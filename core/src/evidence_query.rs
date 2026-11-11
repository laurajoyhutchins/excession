//! Read-only query over independent, source-bound evidence receipts.
//!
//! This intentionally does not rewrite a compiler declaration, a static
//! candidate or a runtime witness into the same *verified dependency edge*.
//! Git authenticates source objects, not untrusted tool execution claims.
use crate::git;
use crate::json::{self, Value, obj};
use crate::{Result, native_boundary, refuse, syntax_resolution};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

const MAX_RECEIPT_BYTES: u64 = 32 * 1024 * 1024;
const MAX_RUNTIME_WITNESSES: usize = 10_000;

fn read(path: &Path) -> Result<Value> {
    let size = fs::metadata(path)
        .map_err(|e| format!("evidence input metadata: {e}"))?
        .len();
    if size > MAX_RECEIPT_BYTES {
        return refuse("evidence input larger than 32 MiB");
    }
    json::parse(&fs::read(path).map_err(|e| format!("evidence input read: {e}"))?)
}

fn identity<'a>(entries: &'a BTreeMap<String, git::Entry>, path: &str) -> Result<&'a git::Entry> {
    git::validate_path(path)?;
    let entry = entries
        .get(path)
        .ok_or_else(|| format!("source path not in pinned Git tree: {path}"))?;
    if entry.kind != "blob" || !matches!(entry.mode.as_str(), "100644" | "100755") {
        return refuse(format!("source is not a regular pinned Git blob: {path}"));
    }
    Ok(entry)
}

fn pinned(entries: &BTreeMap<String, git::Entry>, path: &str, blob: &str) -> Result<()> {
    if identity(entries, path)?.oid != git::oid(blob)? {
        return refuse(format!("source blob differs from pinned Git tree: {path}"));
    }
    Ok(())
}

fn same_revision(report: &Value, commit: &str, tree: &str) -> Result<()> {
    if report.get("source_commit")?.str()? != commit || report.get("source_tree")?.str()? != tree {
        return refuse("evidence refers to another Git commit/tree");
    }
    Ok(())
}

fn native_inputs(
    entries: &BTreeMap<String, git::Entry>,
    declaration: &Value,
    commit: &str,
    tree: &str,
    focus: &str,
) -> Result<(Vec<Value>, Value)> {
    native_boundary::declaration_preflight(declaration)?;
    let assumptions = declaration.get("assumptions")?;
    // Generic native declarations without source-commit/tree cannot be used
    // in a pinned cross-evidence query. The Cargo-depfile adapter supplies both.
    if assumptions.get("source-commit")?.str()? != commit
        || assumptions.get("source-tree")?.str()? != tree
    {
        return refuse("native declaration source commit/tree is unbound or stale");
    }
    let subject = declaration.get("subject")?.str()?;
    identity(entries, subject)?;
    let mut all = Vec::new();
    let mut focus_is_input = false;
    for item in declaration.get("inputs")?.array_ref()? {
        let path = item.get("path")?.str()?;
        let role = item.get("role")?.str()?;
        let entry = identity(entries, path)?;
        if path == focus {
            focus_is_input = true;
        }
        all.push(obj(&[
            ("path", Value::string(path)),
            ("role", Value::string(role)),
            ("blob", Value::string(&entry.oid)),
            ("mode", Value::string(&entry.mode)),
        ]));
    }
    let relevant = if subject == focus {
        all
    } else if focus_is_input {
        all.into_iter()
            .filter(|item| item.get("path").ok().and_then(|x| x.str().ok()) == Some(focus))
            .collect()
    } else {
        Vec::new()
    };
    let receipt = obj(&[
        ("declaration_id", Value::string(declaration.sha256())),
        ("subject", Value::string(subject)),
        ("language", declaration.get("language")?.clone()),
        ("coverage", Value::string("DECLARED_INPUTS_ONLY")),
        ("provenance", Value::string("UNATTESTED_NATIVE_DECLARATION")),
        ("selected_inputs", Value::number(relevant.len())),
        // Raw producer assumptions stay available; no invented toolchain trust.
        ("assumptions", declaration.get("assumptions")?.clone()),
    ]);
    Ok((relevant, receipt))
}

fn runtime_witnesses(
    entries: &BTreeMap<String, git::Entry>,
    report: &Value,
    commit: &str,
    tree: &str,
    focus: &str,
) -> Result<(Vec<Value>, Vec<Value>, Value)> {
    if report.get("schema")?.str()? != "excession.python-runtime-witness.v1"
        || report.get("claim")?.str()? != "OBSERVED_RUNTIME_ONLY"
    {
        return refuse("unsupported runtime witness report contract");
    }
    same_revision(report, commit, tree)?;
    let mut body = report.entries()?.clone();
    let stored_id = body
        .remove("receipt_sha256")
        .ok_or("runtime report missing receipt digest")?
        .str()?
        .to_owned();
    let actual_id = Value::Object(body).sha256();
    if actual_id != stored_id {
        return refuse("runtime witness report digest differs from content");
    }
    let entrypoint = report.get("entrypoint")?;
    let entrypath = entrypoint.get("path")?.str()?;
    pinned(entries, entrypath, entrypoint.get("blob_oid")?.str()?)?;
    if identity(entries, entrypath)?.mode != entrypoint.get("mode")?.str()? {
        return refuse("runtime entrypoint Git mode differs from receipt");
    }
    let include_execution = focus == entrypath;
    let witnessed = report.get("witnesses")?.array_ref()?;
    if witnessed.len() > MAX_RUNTIME_WITNESSES {
        return refuse("runtime witness count exceeds 10,000");
    }
    let mut selected = Vec::new();
    for item in witnessed {
        let source = item.get("source")?;
        let status = source.get("status")?.str()?;
        if status == "GIT_BOUND" {
            let path = source.get("path")?.str()?;
            pinned(entries, path, source.get("blob_oid")?.str()?)?;
            if identity(entries, path)?.mode != source.get("mode")?.str()? {
                return refuse("runtime module Git mode differs from receipt");
            }
        }
        // No assertion about a caller's source path can be inferred from a
        // Python module-name string or "<importlib-caller>".
        let matches_target = status == "GIT_BOUND" && source.get("path")?.str()? == focus;
        if include_execution || matches_target {
            selected.push(obj(&[
                ("witness", item.clone()),
                (
                    "selection",
                    Value::string(if matches_target {
                        "PINNED_TARGET_PATH"
                    } else {
                        "EXECUTION_CONTEXT_ONLY"
                    }),
                ),
            ]));
        }
    }
    let gaps = if include_execution || !selected.is_empty() {
        report.get("gaps")?.array_ref()?.to_vec()
    } else {
        Vec::new()
    };
    let metadata = obj(&[
        ("receipt_id", Value::string(stored_id)),
        ("entrypoint", entrypoint.clone()),
        ("execution", report.get("execution")?.clone()),
        ("interpreter", report.get("interpreter")?.clone()),
        ("claim", Value::string("OBSERVED_RUNTIME_ONLY")),
        (
            "provenance",
            Value::string("SELF_HASHED_UNATTESTED_EXECUTION"),
        ),
    ]);
    Ok((selected, gaps, metadata))
}

/// Query one pinned Git source path across separately typed evidence records.
///
/// Missing receipts are "NOT_SUPPLIED", not an assertion of no dependency.
/// Static evidence is replayed against source; compiler claims are checked
/// against Git paths and provenance, but their execution is not attested.
/// Runtime receipts are self-consistency checked, not independently re-executed.
pub fn query(
    repo: &Path,
    revision: &str,
    focus: &str,
    syntax_file: &Path,
    resolution_file: &Path,
    native_file: Option<&Path>,
    runtime_file: Option<&Path>,
) -> Result<Value> {
    git::verify_repo(repo)?;
    git::validate_path(focus)?;
    let commit = git::rev(repo, revision)?;
    let tree = git::rev(repo, &format!("{commit}^{{tree}}"))?;
    let entries = git::entries(repo, &commit)?
        .into_iter()
        .map(|entry| (entry.path.clone(), entry))
        .collect::<BTreeMap<_, _>>();
    let focus_entry = identity(&entries, focus)?;

    // This is a full source replay, not merely a schema/digest check.
    syntax_resolution::audit(repo, syntax_file, resolution_file)?;
    let resolution = read(resolution_file)?;
    same_revision(&resolution, &commit, &tree)?;
    let syntax = read(syntax_file)?;
    same_revision(&syntax, &commit, &tree)?;

    let mut static_candidates = Vec::new();
    let mut unresolved = Vec::new();
    let mut source_gaps = Vec::new();
    for edge in resolution.get("candidate_edges")?.array_ref()? {
        let from = edge.get("source_path")?.str()?;
        let to = edge.get("target_path")?.str()?;
        if from != focus && to != focus {
            continue;
        }
        pinned(&entries, from, edge.get("source_blob")?.str()?)?;
        pinned(&entries, to, edge.get("target_blob")?.str()?)?;
        static_candidates.push(edge.clone());
    }
    for item in resolution.get("unresolved_imports")?.array_ref()? {
        if item.get("source_path")?.str()? == focus {
            pinned(&entries, focus, item.get("source_blob")?.str()?)?;
            unresolved.push(item.clone());
        }
    }
    for gap in resolution.get("skipped_sources")?.array_ref()? {
        if gap.get("source_path")?.str()? == focus {
            source_gaps.push(gap.clone());
        }
    }

    let (compiler_inputs, native_receipt) = if let Some(path) = native_file {
        native_inputs(&entries, &read(path)?, &commit, &tree, focus)?
    } else {
        (Vec::new(), Value::Null)
    };
    let (runtime, runtime_gaps, runtime_receipt) = if let Some(path) = runtime_file {
        runtime_witnesses(&entries, &read(path)?, &commit, &tree, focus)?
    } else {
        (Vec::new(), Vec::new(), Value::Null)
    };

    let counts = obj(&[
        ("static_candidates", Value::number(static_candidates.len())),
        ("unresolved_static_imports", Value::number(unresolved.len())),
        ("scoped_source_gaps", Value::number(source_gaps.len())),
        (
            "compiler_declared_inputs",
            Value::number(compiler_inputs.len()),
        ),
        ("runtime_witness_records", Value::number(runtime.len())),
        ("runtime_gaps", Value::number(runtime_gaps.len())),
    ]);
    let payload = obj(&[
        ("schema", Value::string("excession.evidence-query.v1")),
        ("source_commit", Value::string(commit)),
        ("source_tree", Value::string(tree)),
        (
            "focus",
            obj(&[
                ("path", Value::string(focus)),
                ("blob", Value::string(&focus_entry.oid)),
                ("mode", Value::string(&focus_entry.mode)),
            ]),
        ),
        (
            "static_replay",
            Value::string("REGENERATED_FROM_PINNED_GIT"),
        ),
        (
            "static_resolution_id",
            resolution.get("resolution_id")?.clone(),
        ),
        ("static_candidates", Value::array(static_candidates)),
        ("unresolved_imports", Value::array(unresolved)),
        ("source_gaps", Value::array(source_gaps)),
        ("compiler_inputs", Value::array(compiler_inputs)),
        ("compiler_receipt", native_receipt),
        ("runtime_witnesses", Value::array(runtime)),
        ("runtime_gaps", Value::array(runtime_gaps)),
        ("runtime_receipt", runtime_receipt),
        ("counts", counts),
        (
            "limits",
            Value::array(vec![
                Value::string("SOURCE_REPLAY_DOES_NOT_PROVE_ALL_DEPENDENCIES"),
                Value::string("COMPILER_DECLARATIONS_ARE_UNATTESTED_INPUT_SETS_NOT_IMPORT_EDGES"),
                Value::string("RUNTIME_OBSERVATIONS_ARE_NOT_COMPLETE_OR_INDEPENDENTLY_ATTESTED"),
                Value::string("ABSENCE_OF_OBSERVATION_IS_NEVER_ABSENCE_OF_DEPENDENCY"),
            ]),
        ),
    ]);
    let id = payload.sha256();
    let mut fields = payload.entries()?.clone();
    fields.insert("query_id".into(), Value::string(id));
    Ok(Value::Object(fields))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git_file(path: &str, oid: &str) -> git::Entry {
        git::Entry {
            path: path.into(),
            mode: "100644".into(),
            kind: "blob".into(),
            oid: oid.into(),
        }
    }
    #[test]
    fn native_claims_need_matching_pinned_revision_and_real_git_paths() {
        let mut entries = BTreeMap::new();
        entries.insert(
            "src/main.rs".into(),
            git_file("src/main.rs", &"a".repeat(40)),
        );
        entries.insert(
            "src/util.rs".into(),
            git_file("src/util.rs", &"b".repeat(40)),
        );
        let base = obj(&[
            ("schema", Value::string("excession.input-declaration.v1")),
            ("coverage", Value::string("DECLARED_INPUTS_ONLY")),
            ("language", Value::string("rust")),
            ("subject", Value::string("src/main.rs")),
            (
                "inputs",
                Value::array(vec![
                    obj(&[
                        ("path", Value::string("src/main.rs")),
                        ("role", Value::string("source")),
                    ]),
                    obj(&[
                        ("path", Value::string("src/util.rs")),
                        ("role", Value::string("source")),
                    ]),
                ]),
            ),
            (
                "assumptions",
                obj(&[
                    (
                        "dependency-discovery",
                        Value::string("excession-toolchain-native-v1"),
                    ),
                    ("toolchain-version", Value::string("rustc test")),
                    ("toolchain-sha256", Value::string("f".repeat(64))),
                    ("platform", Value::string("linux-x86_64")),
                    ("source-commit", Value::string("c".repeat(40))),
                    ("source-tree", Value::string("d".repeat(40))),
                ]),
            ),
        ]);
        let (paths, provenance) = native_inputs(
            &entries,
            &base,
            &"c".repeat(40),
            &"d".repeat(40),
            "src/main.rs",
        )
        .unwrap();
        assert_eq!(paths.len(), 2);
        assert_eq!(
            provenance.get("provenance").unwrap().str().unwrap(),
            "UNATTESTED_NATIVE_DECLARATION"
        );
        let (target, _) = native_inputs(
            &entries,
            &base,
            &"c".repeat(40),
            &"d".repeat(40),
            "src/util.rs",
        )
        .unwrap();
        assert_eq!(target.len(), 1);
        assert!(
            native_inputs(
                &entries,
                &base,
                &"e".repeat(40),
                &"d".repeat(40),
                "src/main.rs"
            )
            .is_err()
        );
    }

    #[test]
    fn self_hashed_runtime_record_is_not_execution_attestation() {
        let mut entries = BTreeMap::new();
        entries.insert("script.py".into(), git_file("script.py", &"a".repeat(40)));
        entries.insert(
            "pkg/plugin.py".into(),
            git_file("pkg/plugin.py", &"b".repeat(40)),
        );
        let payload = obj(&[
            (
                "schema",
                Value::string("excession.python-runtime-witness.v1"),
            ),
            ("claim", Value::string("OBSERVED_RUNTIME_ONLY")),
            ("source_commit", Value::string("c".repeat(40))),
            ("source_tree", Value::string("d".repeat(40))),
            (
                "entrypoint",
                obj(&[
                    ("path", Value::string("script.py")),
                    ("blob_oid", Value::string("a".repeat(40))),
                    ("mode", Value::string("100644")),
                ]),
            ),
            (
                "witnesses",
                Value::array(vec![obj(&[
                    ("module", Value::string("pkg.plugin")),
                    (
                        "source",
                        obj(&[
                            ("status", Value::string("GIT_BOUND")),
                            ("path", Value::string("pkg/plugin.py")),
                            ("blob_oid", Value::string("b".repeat(40))),
                            ("mode", Value::string("100644")),
                        ]),
                    ),
                ])]),
            ),
            (
                "gaps",
                Value::array(vec![obj(&[("kind", Value::string("COVERAGE_UNPROVEN"))])]),
            ),
            ("execution", obj(&[("exit_code", Value::number(0))])),
            ("interpreter", obj(&[("version", Value::string("3.13"))])),
        ]);
        let mut full = payload.entries().unwrap().clone();
        full.insert("receipt_sha256".into(), Value::string(payload.sha256()));
        let report = Value::Object(full);
        let (observed, gaps, provenance) = runtime_witnesses(
            &entries,
            &report,
            &"c".repeat(40),
            &"d".repeat(40),
            "script.py",
        )
        .unwrap();
        assert_eq!(observed.len(), 1);
        assert_eq!(gaps.len(), 1);
        assert_eq!(
            provenance.get("provenance").unwrap().str().unwrap(),
            "SELF_HASHED_UNATTESTED_EXECUTION"
        );
        let (target, _, _) = runtime_witnesses(
            &entries,
            &report,
            &"c".repeat(40),
            &"d".repeat(40),
            "pkg/plugin.py",
        )
        .unwrap();
        assert_eq!(target.len(), 1);
        let mut forged = report.entries().unwrap().clone();
        forged.insert("source_tree".into(), Value::string("e".repeat(40)));
        assert!(
            runtime_witnesses(
                &entries,
                &Value::Object(forged),
                &"c".repeat(40),
                &"d".repeat(40),
                "script.py"
            )
            .is_err()
        );
    }
}
