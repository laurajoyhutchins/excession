//! Independent Git-byte comparison with facet-scoped coverage.
//! Identical source bytes do not establish behavioral equivalence or absent edges.
use crate::evidence::identity::SourceRef;
use crate::evidence_coverage::Coverage;
use crate::json::{Value, obj};
use crate::{Result, digest, git, refuse};
use std::path::Path;

const MAX_BYTES: usize = 2 * 1024 * 1024;

fn source(value: &SourceRef) -> Value {
    obj(&[
        ("source_commit", Value::string(&value.commit)),
        ("source_tree", Value::string(&value.tree)),
        ("path", Value::string(&value.path)),
        ("blob", Value::string(&value.blob)),
        ("mode", Value::string(&value.mode)),
    ])
}

fn read(repo: &Path, source: &SourceRef) -> Result<Vec<u8>> {
    source.verify_git(repo)?;
    let mut reader = git::BlobReader::open(repo)?;
    let bytes = reader.read_capped(&source.blob, MAX_BYTES)?;
    reader.finish()?;
    Ok(bytes)
}

/// Compare exact source bytes under two independently replayed Git identities.
/// The two scoped producer-coverage records are *context*, never a closed-world
/// proof about calls, imports, artifacts, or observable program behavior.
pub fn compare(
    left_repo: &Path,
    left: &SourceRef,
    left_coverage: &Coverage,
    right_repo: &Path,
    right: &SourceRef,
    right_coverage: &Coverage,
) -> Result<Value> {
    if left_coverage.source_commit != left.commit || right_coverage.source_commit != right.commit {
        return refuse("coverage source commit does not match comparison subject");
    }
    if left_coverage.facet != right_coverage.facet {
        return refuse("comparison requires the same evidence facet on both sides");
    }
    let left_status = left_coverage.status()?;
    let right_status = right_coverage.status()?;
    let left_bytes = read(left_repo, left)?;
    let right_bytes = read(right_repo, right)?;
    let same_content = left_bytes == right_bytes;
    let comparable_scope = left_coverage.selection == right_coverage.selection
        && left_coverage.configuration == right_coverage.configuration
        && left_coverage.configuration_resolved
        && right_coverage.configuration_resolved;
    let mut record = obj(&[
        ("schema", Value::string("excession.source-comparison.v1")),
        ("left", source(left)),
        ("right", source(right)),
        ("facet", Value::string(left_coverage.facet.as_str())),
        ("left_coverage", left_coverage.report()?),
        ("right_coverage", right_coverage.report()?),
        ("left_coverage_status", Value::string(left_status.as_str())),
        (
            "right_coverage_status",
            Value::string(right_status.as_str()),
        ),
        ("coverage_scope_comparable", Value::Bool(comparable_scope)),
        ("left_byte_count", Value::number(left_bytes.len())),
        ("right_byte_count", Value::number(right_bytes.len())),
        ("left_sha256", Value::string(digest::sha256(&left_bytes))),
        ("right_sha256", Value::string(digest::sha256(&right_bytes))),
        (
            "content_relation",
            Value::string(if same_content {
                "IDENTICAL_BYTES"
            } else {
                "DIFFERENT_BYTES"
            }),
        ),
        ("mode_equal", Value::Bool(left.mode == right.mode)),
        ("relationship_absence_proven", Value::Bool(false)),
        ("behavioral_equivalence", Value::string("NOT_ESTABLISHED")),
        ("verification", Value::string("INDEPENDENT_GIT_BYTES_ONLY")),
    ]);
    let id = record.sha256();
    record
        .entries_mut()?
        .insert("report_id".into(), Value::string(id));
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence_coverage::{Accounting, Facet, Status};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture {
        repo: PathBuf,
        base: String,
        unchanged: String,
        changed: String,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.repo);
        }
    }
    fn git(repo: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }
    impl Fixture {
        fn new() -> Self {
            let repo = std::env::temp_dir().join(format!(
                "excession-git-compare-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&repo).unwrap();
            git(&repo, &["init", "-q"]);
            git(&repo, &["config", "user.email", "test@example.invalid"]);
            git(&repo, &["config", "user.name", "Contract Test"]);
            fs::write(repo.join("sample.rs"), "fn original() {}\n").unwrap();
            git(&repo, &["add", "sample.rs"]);
            git(&repo, &["commit", "-qm", "initial"]);
            let base = git(&repo, &["rev-parse", "HEAD"]);
            fs::write(repo.join("OTHER.md"), "unrelated\n").unwrap();
            git(&repo, &["add", "OTHER.md"]);
            git(&repo, &["commit", "-qm", "unrelated"]);
            let unchanged = git(&repo, &["rev-parse", "HEAD"]);
            fs::write(repo.join("sample.rs"), "fn updated() {}\n").unwrap();
            git(&repo, &["add", "sample.rs"]);
            git(&repo, &["commit", "-qm", "source change"]);
            let changed = git(&repo, &["rev-parse", "HEAD"]);
            Self {
                repo,
                base,
                unchanged,
                changed,
            }
        }
        fn source(&self, commit: &str) -> SourceRef {
            SourceRef::from_git(&self.repo, commit, "sample.rs").unwrap()
        }
        fn coverage(&self, commit: &str) -> Coverage {
            Coverage {
                source_commit: commit.into(),
                producer_receipt_id: "a".repeat(64),
                facet: Facet::ScipReferences,
                selection: "sample.rs".into(),
                configuration: "rust-analyzer@1.99.1".into(),
                configuration_resolved: true,
                declared_total: None,
                accounting: Accounting {
                    examined: 1,
                    unsupported: 0,
                    excluded: 0,
                    unresolved: 1,
                    truncated: false,
                },
                limitations: vec!["partial native index".into()],
                unsupported_facet: false,
            }
        }
    }
    #[test]
    fn real_git_bytes_equal_or_different_without_behavioral_promotion() {
        let f = Fixture::new();
        let source_a = f.source(&f.base);
        let source_b = f.source(&f.unchanged);
        let same = compare(
            &f.repo,
            &source_a,
            &f.coverage(&f.base),
            &f.repo,
            &source_b,
            &f.coverage(&f.unchanged),
        )
        .unwrap();
        assert_eq!(
            same.get("content_relation").unwrap().str().unwrap(),
            "IDENTICAL_BYTES"
        );
        assert_eq!(
            same.get("left_sha256").unwrap(),
            same.get("right_sha256").unwrap()
        );
        assert_eq!(
            same.get("left_coverage_status").unwrap().str().unwrap(),
            Status::Partial.as_str()
        );
        assert!(
            !same
                .get("relationship_absence_proven")
                .unwrap()
                .bool()
                .unwrap()
        );
        let source_c = f.source(&f.changed);
        let different = compare(
            &f.repo,
            &source_a,
            &f.coverage(&f.base),
            &f.repo,
            &source_c,
            &f.coverage(&f.changed),
        )
        .unwrap();
        assert_eq!(
            different.get("content_relation").unwrap().str().unwrap(),
            "DIFFERENT_BYTES"
        );
        assert_eq!(
            different
                .get("behavioral_equivalence")
                .unwrap()
                .str()
                .unwrap(),
            "NOT_ESTABLISHED"
        );
        assert_ne!(
            same.get("report_id").unwrap(),
            different.get("report_id").unwrap()
        );

        let schema: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/evidence-core/comparison.v1.schema.json"
        ))
        .unwrap();
        let validator = jsonschema::draft202012::new(&schema).unwrap();
        let output: serde_json::Value = serde_json::from_str(&different.canonical()).unwrap();
        assert!(validator.is_valid(&output));
    }
    #[test]
    fn forged_source_wrong_facet_and_wrong_coverage_refuse() {
        let f = Fixture::new();
        let good = f.source(&f.base);
        let changed = f.source(&f.changed);
        let mut bad = changed.clone();
        bad.blob = "f".repeat(40);
        assert!(
            compare(
                &f.repo,
                &good,
                &f.coverage(&f.base),
                &f.repo,
                &bad,
                &f.coverage(&f.changed),
            )
            .is_err()
        );
        let mut scope = f.coverage(&f.changed);
        scope.facet = Facet::BuildInputs;
        assert!(
            compare(
                &f.repo,
                &good,
                &f.coverage(&f.base),
                &f.repo,
                &changed,
                &scope,
            )
            .is_err()
        );
        scope = f.coverage(&f.changed);
        scope.source_commit = f.base.clone();
        assert!(
            compare(
                &f.repo,
                &good,
                &f.coverage(&f.base),
                &f.repo,
                &changed,
                &scope,
            )
            .is_err()
        );
    }
    #[test]
    fn partial_and_disjoint_scope_never_prove_absence() {
        let f = Fixture::new();
        let left = f.source(&f.base);
        let right = f.source(&f.unchanged);
        let mut right_scope = f.coverage(&f.unchanged);
        right_scope.configuration = "different".into();
        let report = compare(
            &f.repo,
            &left,
            &f.coverage(&f.base),
            &f.repo,
            &right,
            &right_scope,
        )
        .unwrap();
        assert!(
            !report
                .get("coverage_scope_comparable")
                .unwrap()
                .bool()
                .unwrap()
        );
        assert!(
            !report
                .get("relationship_absence_proven")
                .unwrap()
                .bool()
                .unwrap()
        );
    }
    #[test]
    fn bounded_source_bytes_refuse_oversized_input() {
        let f = Fixture::new();
        fs::write(f.repo.join("sample.rs"), vec![b'x'; MAX_BYTES + 1]).unwrap();
        git(&f.repo, &["add", "sample.rs"]);
        git(&f.repo, &["commit", "-qm", "oversized"]);
        let large = git(&f.repo, &["rev-parse", "HEAD"]);
        assert!(
            compare(
                &f.repo,
                &f.source(&f.base),
                &f.coverage(&f.base),
                &f.repo,
                &f.source(&large),
                &f.coverage(&large),
            )
            .is_err()
        );
    }
}
