//! Typed source/artifact/producer identities with explicit trust boundaries.
//! Shape checks, content integrity and Git source replay are separate operations.
use crate::{Result, digest, git, refuse};
use std::path::Path;

fn sha256_hex(value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return refuse("expected 64 lowercase SHA-256 hex characters");
    }
    Ok(())
}

fn required(value: &str, label: &str, limit: usize) -> Result<()> {
    if value.is_empty() || value.len() > limit || value.contains('\0') {
        return refuse(format!("invalid {label}"));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceRef {
    pub commit: String,
    pub tree: String,
    pub path: String,
    pub blob: String,
    pub mode: String,
}

impl SourceRef {
    // A tree object and an annotated tag are not valid source commit identities.
    // Resolve through Git, then require the supplied OID itself to be a commit.
    fn verify_commit(repo: &Path, commit: &str) -> Result<()> {
        git::oid(commit)?;
        if git::rev(repo, &format!("{commit}^{{commit}}"))? != commit {
            return refuse("source identity must name a Git commit, not a tag");
        }
        Ok(())
    }

    /// Construct one canonical source identity exclusively from a pinned Git
    /// revision. No working-tree paths or caller-provided blobs are trusted.
    /// This authenticates file identity, not the producer's interpretation.
    pub fn from_git(repo: &Path, commit: &str, path: &str) -> Result<Self> {
        git::verify_repo(repo)?;
        Self::verify_commit(repo, commit)?;
        git::validate_path(path)?;
        let tree = git::rev(repo, &format!("{commit}^{{tree}}"))?;
        let entry = git::entry_at(repo, commit, path)?;
        if entry.kind != "blob" || !matches!(entry.mode.as_str(), "100644" | "100755") {
            return refuse("evidence source requires tracked regular file mode");
        }
        let source = Self {
            commit: commit.to_owned(),
            tree,
            path: path.to_owned(),
            blob: entry.oid,
            mode: entry.mode,
        };
        // These are the exact values just read from Git. Calling verify_git
        // here would query the same commit, tree, and entry a second time.
        // The separate verify_git method remains for untrusted supplied refs.
        source.validate()?;
        Ok(source)
    }

    pub fn validate(&self) -> Result<()> {
        git::oid(&self.commit)?;
        git::oid(&self.tree)?;
        git::oid(&self.blob)?;
        git::validate_path(&self.path)?;
        if !matches!(self.mode.as_str(), "100644" | "100755") {
            return refuse("evidence source requires tracked regular file mode");
        }
        Ok(())
    }

    /// Independently bind the claimed source to the given SHA-1 Git repository.
    /// This does not establish any producer's accuracy or that a tool executed.
    pub fn verify_git(&self, repo: &Path) -> Result<()> {
        self.validate()?;
        git::verify_repo(repo)?;
        Self::verify_commit(repo, &self.commit)?;
        let actual_tree = git::rev(repo, &format!("{}^{{tree}}", self.commit))?;
        if actual_tree != self.tree {
            return refuse("evidence source commit/tree identity mismatch");
        }
        let entry = git::entry_at(repo, &self.commit, &self.path)?;
        if entry.kind != "blob" || entry.mode != self.mode || entry.oid != self.blob {
            return refuse("evidence source file mode or identity mismatch");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpanEncoding {
    Utf8,
    Utf16,
    Utf32,
    LineOnly,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub first_line: u64,
    pub last_line: u64,
    pub encoding: SpanEncoding,
}
impl Span {
    fn validate(&self) -> Result<()> {
        if self.first_line == 0 || self.last_line < self.first_line {
            return refuse("invalid source span line range");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Subject {
    Source(SourceRef),
    Symbol {
        source: SourceRef,
        scip_symbol: String,
        span: Option<Span>,
    },
    Artifact {
        sha256: String,
        size_bytes: u64,
    },
    Execution {
        artifact_sha256: String,
        execution_receipt_sha256: String,
    },
    Configuration {
        sha256: String,
    },
}

fn field(bytes: &mut Vec<u8>, value: &str) {
    bytes.extend_from_slice(&(value.len() as u64).to_be_bytes());
    bytes.extend_from_slice(value.as_bytes());
}
fn source_fields(bytes: &mut Vec<u8>, source: &SourceRef) {
    for value in [
        &source.commit,
        &source.tree,
        &source.path,
        &source.blob,
        &source.mode,
    ] {
        field(bytes, value);
    }
}
impl Subject {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Source(src) => src.validate(),
            Self::Symbol {
                source,
                scip_symbol,
                span,
            } => {
                source.validate()?;
                required(scip_symbol, "original SCIP symbol", 16_384)?;
                if let Some(span) = span {
                    span.validate()?;
                }
                Ok(())
            }
            Self::Artifact { sha256, .. } | Self::Configuration { sha256 } => sha256_hex(sha256),
            Self::Execution {
                artifact_sha256,
                execution_receipt_sha256,
            } => {
                sha256_hex(artifact_sha256)?;
                sha256_hex(execution_receipt_sha256)
            }
        }
    }

    /// Domain-separated content identity for the exact typed subject description.
    /// This is a digest, not proof of a semantic or cross-revision match.
    pub fn id(&self) -> Result<String> {
        self.validate()?;
        let mut bytes = b"excession.subject.v1\0".to_vec();
        match self {
            Self::Source(src) => {
                bytes.push(1);
                source_fields(&mut bytes, src);
            }
            Self::Symbol {
                source,
                scip_symbol,
                span,
            } => {
                bytes.push(2);
                source_fields(&mut bytes, source);
                field(&mut bytes, scip_symbol);
                if let Some(span) = span {
                    bytes.push(1);
                    bytes.extend_from_slice(&span.first_line.to_be_bytes());
                    bytes.extend_from_slice(&span.last_line.to_be_bytes());
                    bytes.push(match span.encoding {
                        SpanEncoding::Utf8 => 1,
                        SpanEncoding::Utf16 => 2,
                        SpanEncoding::Utf32 => 3,
                        SpanEncoding::LineOnly => 4,
                    });
                } else {
                    bytes.push(0);
                }
            }
            Self::Artifact { sha256, size_bytes } => {
                bytes.push(3);
                field(&mut bytes, sha256);
                bytes.extend_from_slice(&size_bytes.to_be_bytes());
            }
            Self::Execution {
                artifact_sha256,
                execution_receipt_sha256,
            } => {
                bytes.push(4);
                field(&mut bytes, artifact_sha256);
                field(&mut bytes, execution_receipt_sha256);
            }
            Self::Configuration { sha256 } => {
                bytes.push(5);
                field(&mut bytes, sha256);
            }
        }
        Ok(digest::sha256(&bytes))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProducerProvenance {
    SelfHashedUnattested,
    SourceReplayed,
    ExternalAttestationClaim { attestation_sha256: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProducerReceipt {
    pub producer: String,
    pub version: String,
    pub payload_sha256: String,
    pub input_sha256: String,
    pub provenance: ProducerProvenance,
}
impl ProducerReceipt {
    pub fn validate(&self) -> Result<()> {
        required(&self.producer, "producer", 256)?;
        required(&self.version, "producer version", 256)?;
        sha256_hex(&self.payload_sha256)?;
        sha256_hex(&self.input_sha256)?;
        if let ProducerProvenance::ExternalAttestationClaim { attestation_sha256 } =
            &self.provenance
        {
            sha256_hex(attestation_sha256)?;
        }
        Ok(())
    }

    /// Checks only byte-level integrity. Does not independently verify the producer,
    /// source replay, attestation, execution or truth of the observations.
    pub fn verify_payload(&self, raw: &[u8]) -> Result<()> {
        self.validate()?;
        if digest::sha256(raw) != self.payload_sha256 {
            return refuse("producer evidence payload digest mismatch");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn source() -> SourceRef {
        SourceRef {
            commit: "a".repeat(40),
            tree: "b".repeat(40),
            path: "src/lib.rs".into(),
            blob: "c".repeat(40),
            mode: "100644".into(),
        }
    }
    #[test]
    fn typed_subject_ids_are_stable_and_domain_separated() {
        let src = Subject::Source(source());
        assert_eq!(src.id().unwrap(), src.id().unwrap());
        let sym = Subject::Symbol {
            source: source(),
            scip_symbol: "rust-analyzer cargo example src/foo".into(),
            span: None,
        };
        assert_ne!(src.id().unwrap(), sym.id().unwrap());
        let art = Subject::Artifact {
            sha256: "d".repeat(64),
            size_bytes: 123,
        };
        assert_ne!(src.id().unwrap(), art.id().unwrap());
        assert_ne!(
            art.id().unwrap(),
            Subject::Artifact {
                sha256: "d".repeat(64),
                size_bytes: 124
            }
            .id()
            .unwrap()
        );
    }
    #[test]
    fn artifact_and_execution_do_not_require_fabricated_git_refs() {
        assert!(
            Subject::Artifact {
                sha256: "d".repeat(64),
                size_bytes: 0
            }
            .validate()
            .is_ok()
        );
        assert!(
            Subject::Execution {
                artifact_sha256: "d".repeat(64),
                execution_receipt_sha256: "e".repeat(64)
            }
            .validate()
            .is_ok()
        );
        assert!(
            Subject::Configuration {
                sha256: "f".repeat(64)
            }
            .validate()
            .is_ok()
        );
    }
    #[test]
    fn invalid_paths_modes_and_spans_refused() {
        let mut src = source();
        src.path = "../unsafe".into();
        assert!(src.validate().is_err());
        let mut src = source();
        src.mode = "120000".into();
        assert!(src.validate().is_err());
        let mut src = source();
        src.commit = "A".repeat(40);
        assert!(src.validate().is_err());
        let sym = Subject::Symbol {
            source: source(),
            scip_symbol: "x".into(),
            span: Some(Span {
                first_line: 3,
                last_line: 2,
                encoding: SpanEncoding::Utf16,
            }),
        };
        assert!(sym.validate().is_err());
    }
    #[test]
    fn self_hash_not_attestation_and_tamper_refusal() {
        let receipt = ProducerReceipt {
            producer: "scip-rust".into(),
            version: "1".into(),
            payload_sha256: digest::sha256(b"observed"),
            input_sha256: "a".repeat(64),
            provenance: ProducerProvenance::SelfHashedUnattested,
        };
        receipt.verify_payload(b"observed").unwrap();
        assert!(receipt.verify_payload(b"forged").is_err());
        assert_eq!(receipt.provenance, ProducerProvenance::SelfHashedUnattested);
        let forged = ProducerReceipt {
            provenance: ProducerProvenance::ExternalAttestationClaim {
                attestation_sha256: "invalid".into(),
            },
            ..receipt
        };
        assert!(forged.validate().is_err());
    }
    #[test]
    fn git_replay_rejects_forged_blob_mode_or_tree() {
        let root = std::env::temp_dir().join(format!(
            "excession-evidence-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("src")).unwrap();
        let run = |args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout).unwrap().trim().to_owned()
        };
        run(&["init", "-q"]);
        std::fs::write(root.join("src/lib.rs"), "pub fn f() {}\n").unwrap();
        run(&["add", "src/lib.rs"]);
        run(&[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-qm",
            "test",
        ]);
        let src = SourceRef {
            commit: run(&["rev-parse", "HEAD"]),
            tree: run(&["rev-parse", "HEAD^{tree}"]),
            path: "src/lib.rs".into(),
            blob: run(&["rev-parse", "HEAD:src/lib.rs"]),
            mode: "100644".into(),
        };
        src.verify_git(&root).unwrap();
        assert_eq!(
            SourceRef::from_git(&root, &src.commit, &src.path).unwrap(),
            src,
            "a Git-derived source must retain the original canonical identity"
        );
        assert!(SourceRef::from_git(&root, &src.commit, "../outside").is_err());
        let tree_oid = run(&["rev-parse", "HEAD^{tree}"]);
        assert!(SourceRef::from_git(&root, &tree_oid, "src/lib.rs").is_err());
        let mut bad = src.clone();
        bad.commit = tree_oid;
        assert!(bad.verify_git(&root).is_err());
        let mut bad = src.clone();
        bad.blob = "d".repeat(40);
        assert!(bad.verify_git(&root).is_err());
        let mut bad = src.clone();
        bad.tree = "e".repeat(40);
        assert!(bad.verify_git(&root).is_err());
        let mut bad = src;
        bad.mode = "100755".into();
        assert!(bad.verify_git(&root).is_err());
        let _ = std::fs::remove_dir_all(root);
    }
}
