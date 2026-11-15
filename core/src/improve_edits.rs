//! Replay a native edit producer's byte-exact plan without knowing its language.
//! An edit plan is an unauthenticated proposal, never evidence of safe deletion.
use crate::json::{self, Value, obj};
use crate::{Result, analyze, digest, git, refuse};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

const MAX_PLAN_BYTES: usize = 4 * 1024 * 1024;
const MAX_SOURCE_BYTES: usize = 512 * 1024;
const MAX_EDIT_COUNT: usize = 32;
const MAX_FILE_COUNT: usize = 8;
const MAX_REPLACEMENT_BYTES: usize = 64 * 1024;

pub struct Request<'a> {
    pub repo: &'a Path,
    pub revision: &'a str,
    pub plan: &'a Path,
    pub edits: &'a Path,
    pub output: Option<&'a Path>,
}

fn read(path: &Path) -> Result<Value> {
    let bytes = fs::read(path).map_err(|e| format!("improvement artifact read: {e}"))?;
    if bytes.len() > MAX_PLAN_BYTES {
        return refuse("improvement artifact exceeds 4 MiB");
    }
    json::parse(&bytes)
}

fn selected_paths(plan: &Value, candidate: &str) -> Result<BTreeSet<String>> {
    let mut candidates = plan
        .get("candidates")?
        .array_ref()?
        .iter()
        .filter(|item| item.get("candidate_id").and_then(Value::str).ok() == Some(candidate));
    let found = candidates
        .next()
        .ok_or("edit target absent from improvement plan")?;
    if candidates.next().is_some() {
        return refuse("ambiguous improvement candidate ID");
    }
    let mut paths = BTreeSet::new();
    for location in found.get("finding")?.get("locations")?.array_ref()? {
        let path = location.get("path")?.str()?;
        git::validate_path(path)?;
        paths.insert(path.to_owned());
    }
    if paths.is_empty() {
        return refuse("improvement candidate has no Git-bound paths");
    }
    Ok(paths)
}

#[derive(Clone)]
struct Edit {
    start: usize,
    end: usize,
    replacement: String,
}

struct Changed {
    path: String,
    original_blob: String,
    original: Vec<u8>,
    rewritten: Vec<u8>,
}

fn byte_offset(value: &Value) -> Result<usize> {
    usize::try_from(value.integer()?).map_err(|_| "negative native edit offset".to_owned())
}

fn replay_file(original: &[u8], mut edits: Vec<Edit>) -> Result<Vec<u8>> {
    let source = std::str::from_utf8(original)
        .map_err(|_| "native edit replay currently supports UTF-8 source only".to_owned())?;
    edits.sort_by_key(|edit| (edit.start, edit.end));
    for (index, edit) in edits.iter().enumerate() {
        if edit.start > edit.end
            || edit.end > original.len()
            || !source.is_char_boundary(edit.start)
            || !source.is_char_boundary(edit.end)
        {
            return refuse("native edit crosses a source or UTF-8 boundary");
        }
        if index > 0 && edit.start <= edits[index - 1].end {
            return refuse("native edit ranges overlap or share an ambiguous boundary");
        }
    }
    let mut rewritten = source.to_owned();
    for edit in edits.into_iter().rev() {
        rewritten.replace_range(edit.start..edit.end, &edit.replacement);
    }
    if rewritten.len() > MAX_SOURCE_BYTES {
        return refuse("native edit exceeds 512 KiB rewritten file");
    }
    if rewritten.as_bytes() == original {
        return refuse("native edit has no effect on its source");
    }
    Ok(rewritten.into_bytes())
}

fn prepare(repo: &Path, commit: &str, plan: &Value, native: &Value) -> Result<Vec<Changed>> {
    if native.get("schema")?.str()? != "excession.improvement-native-edits.v1"
        || !native.content_id_matches("edit_plan_id")?
    {
        return refuse("native edit plan schema or content ID is invalid");
    }
    if native.get("source_commit")? != plan.get("source_commit")?
        || native.get("source_tree")? != plan.get("source_tree")?
        || native.get("source_commit")?.str()? != commit
        || native.get("improvement_plan_id")? != plan.get("plan_id")?
    {
        return refuse(
            "native edits are not bound to the selected improvement plan and Git revision",
        );
    }
    let producer = native.get("native_producer")?.str()?;
    if producer.is_empty() || producer.len() > 128 {
        return refuse("native edit producer identity is absent or oversized");
    }
    let allowed = selected_paths(plan, native.get("candidate_id")?.str()?)?;
    let operations = native.get("edits")?.array_ref()?;
    if operations.is_empty() || operations.len() > MAX_EDIT_COUNT {
        return refuse("native edit plan requires 1..32 bounded edits");
    }
    let mut by_path: BTreeMap<String, (String, Vec<Edit>)> = BTreeMap::new();
    for operation in operations {
        let path = operation.get("path")?.str()?;
        git::validate_path(path)?;
        if !allowed.contains(path) {
            return refuse("native edit targets a path outside the source-bound candidate");
        }
        let oid = operation.get("original_blob")?.str()?;
        git::oid(oid)?;
        let replacement = operation.get("replacement")?.str()?;
        if replacement.len() > MAX_REPLACEMENT_BYTES {
            return refuse("native replacement exceeds 64 KiB");
        }
        let edit = Edit {
            start: byte_offset(operation.get("start_byte")?)?,
            end: byte_offset(operation.get("end_byte")?)?,
            replacement: replacement.to_owned(),
        };
        let group = by_path
            .entry(path.to_owned())
            .or_insert_with(|| (oid.to_owned(), Vec::new()));
        if group.0 != oid {
            return refuse("conflicting pinned blob identities for one source");
        }
        group.1.push(edit);
    }
    if by_path.len() > MAX_FILE_COUNT {
        return refuse("native edit touches more than eight source files");
    }
    let mut reader = git::BlobReader::open(repo)?;
    let mut changed = Vec::new();
    for (path, (oid, edits)) in by_path {
        let entry = git::entry_at(repo, commit, &path)?;
        if entry.kind != "blob"
            || !matches!(entry.mode.as_str(), "100644" | "100755")
            || entry.oid != oid
        {
            return refuse("native edit source is not the exact pinned regular Git blob");
        }
        let original = reader.read_capped(&oid, MAX_SOURCE_BYTES)?;
        let rewritten = replay_file(&original, edits)?;
        changed.push(Changed {
            path,
            original_blob: oid,
            original,
            rewritten,
        });
    }
    reader.finish()?;
    Ok(changed)
}

fn apply(repo: &Path, commit: &str, output: &Path, changed: &[Changed]) -> Result<()> {
    let checked = analyze::checked_external_destination(repo, output, "improvement worktree")?;
    if checked.exists() {
        return refuse("improvement worktree destination already exists");
    }
    let output_str = checked.to_str().ok_or("non-UTF-8 output worktree path")?;
    git::command(
        repo,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "worktree",
            "add",
            "--detach",
            output_str,
            commit,
        ],
    )?;
    let result = (|| -> Result<()> {
        let root = checked
            .canonicalize()
            .map_err(|e| format!("resolve candidate worktree: {e}"))?;
        for item in changed {
            let target = checked.join(&item.path);
            let parent = target.parent().ok_or("native edit path has no parent")?;
            if !parent
                .canonicalize()
                .map_err(|e| format!("resolve native edit parent: {e}"))?
                .starts_with(&root)
            {
                return refuse("native edit path escapes through a symlink");
            }
            if fs::read(&target).map_err(|e| format!("read candidate source: {e}"))?
                != item.original
            {
                return refuse("candidate worktree source differs from pinned Git blob");
            }
            fs::write(&target, &item.rewritten)
                .map_err(|e| format!("write candidate source: {e}"))?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        let _ = git::command(
            repo,
            &[
                "-c",
                "core.hooksPath=/dev/null",
                "worktree",
                "remove",
                "--force",
                output_str,
            ],
        );
        return Err(error);
    }
    Ok(())
}

/// Replay edits from a separately produced native tool. Preview runs no code;
/// apply creates an isolated detached worktree and never certifies semantics.
pub fn execute(request: &Request<'_>) -> Result<Value> {
    let commit = git::rev(request.repo, request.revision)?;
    let tree = git::rev(request.repo, &format!("{commit}^{{tree}}"))?;
    let plan = read(request.plan)?;
    if plan.get("schema")?.str()? != "excession.improvement-plan.v1"
        || !plan.content_id_matches("plan_id")?
        || plan.get("source_commit")?.str()? != commit
        || plan.get("source_tree")?.str()? != tree
    {
        return refuse("improvement plan is not bound to this Git commit");
    }
    let native = read(request.edits)?;
    let changed = prepare(request.repo, &commit, &plan, &native)?;
    if let Some(output) = request.output {
        apply(request.repo, &commit, output, &changed)?;
    }
    obj(&[
        (
            "schema",
            Value::string("excession.improvement-native-replay.v1"),
        ),
        (
            "status",
            Value::string(if request.output.is_some() {
                "APPLIED_UNVERIFIED"
            } else {
                "UNVERIFIED_EDIT_PREVIEW"
            }),
        ),
        ("source_commit", Value::string(commit)),
        ("source_tree", Value::string(tree)),
        ("improvement_plan_id", plan.get("plan_id")?.clone()),
        ("edit_plan_id", native.get("edit_plan_id")?.clone()),
        ("native_producer", native.get("native_producer")?.clone()),
        ("candidate_id", native.get("candidate_id")?.clone()),
        ("source_changes", Value::number(changed.len())),
        (
            "changes",
            Value::array(changed.iter().map(|item| {
                obj(&[
                    ("path", Value::string(&item.path)),
                    ("original_blob", Value::string(&item.original_blob)),
                    (
                        "rewritten_sha256",
                        Value::string(digest::sha256(&item.rewritten)),
                    ),
                    (
                        "removed_bytes",
                        Value::number(item.original.len().saturating_sub(item.rewritten.len())),
                    ),
                    (
                        "added_bytes",
                        Value::number(item.rewritten.len().saturating_sub(item.original.len())),
                    ),
                ])
            })),
        ),
        (
            "worktree",
            request
                .output
                .map(|p| Value::string(p.to_string_lossy()))
                .unwrap_or(Value::Null),
        ),
        (
            "obligations",
            Value::array([
                Value::string("NATIVE_PRODUCER_AUTHENTICITY_UNVERIFIED"),
                Value::string("INDEPENDENT_BUILD_TEST_AND_API_VERIFICATION_REQUIRED"),
                Value::string("CONFIGURATION_COVERAGE_UNVERIFIED"),
                Value::string("BUILD_PROVENANCE_AND_REA_EVIDENCE_NOT_ATTESTED"),
            ]),
        ),
        ("ready_to_merge", Value::Bool(false)),
        ("automatic_deletion_authorized", Value::Bool(false)),
    ])
    .stamp_id("report_id")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_offsets_require_unambiguous_valid_utf8() {
        let source = "aéb".as_bytes();
        assert!(
            replay_file(
                source,
                vec![Edit {
                    start: 1,
                    end: 2,
                    replacement: String::new(),
                }]
            )
            .is_err()
        );
        assert!(
            replay_file(
                source,
                vec![
                    Edit {
                        start: 0,
                        end: 1,
                        replacement: String::new()
                    },
                    Edit {
                        start: 1,
                        end: 1,
                        replacement: "x".to_owned()
                    },
                ]
            )
            .is_err()
        );
        assert_eq!(
            replay_file(
                source,
                vec![Edit {
                    start: 1,
                    end: 3,
                    replacement: String::new(),
                }]
            )
            .unwrap(),
            b"ab"
        );
        assert!(
            replay_file(
                source,
                vec![Edit {
                    start: 0,
                    end: 0,
                    replacement: String::new(),
                }]
            )
            .is_err()
        );
    }
}
