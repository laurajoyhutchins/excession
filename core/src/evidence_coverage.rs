//! Facet-scoped accounting over producer-native observations.
//! Coverage completeness never establishes semantic or runtime closure.
use crate::json::{Value, obj};
use crate::{Result, git, refuse};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Facet {
    GitSources,
    StaticImports,
    ScipReferences,
    BuildInputs,
    Calls,
    ArtifactFunctions,
    RuntimeWitnesses,
}
impl Facet {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GitSources => "GIT_SOURCES",
            Self::StaticImports => "IMPORTS",
            Self::ScipReferences => "SCIP_REFERENCES",
            Self::BuildInputs => "BUILD_INPUTS",
            Self::Calls => "CALLS",
            Self::ArtifactFunctions => "ARTIFACT_FUNCTIONS",
            Self::RuntimeWitnesses => "RUNTIME_WITNESSES",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    CompleteDeclaredScope,
    Partial,
    Unknown,
    Unsupported,
}
impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CompleteDeclaredScope => "COMPLETE_DECLARED_SCOPE",
            Self::Partial => "PARTIAL",
            Self::Unknown => "UNKNOWN",
            Self::Unsupported => "UNSUPPORTED",
        }
    }
}

/// Counts refer to the same declared finite population. `unresolved` is a
/// separate semantic gap count, never used to balance missing inventory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Accounting {
    pub examined: usize,
    pub unsupported: usize,
    pub excluded: usize,
    pub unresolved: usize,
    pub truncated: bool,
}

/// An unbounded or unknown universe cannot be promoted to a complete scope.
#[derive(Clone, Debug)]
pub struct Coverage {
    pub source_commit: String,
    pub producer_receipt_id: String,
    pub facet: Facet,
    pub selection: String,
    pub configuration: String,
    pub configuration_resolved: bool,
    pub declared_total: Option<usize>,
    pub accounting: Accounting,
    pub limitations: Vec<String>,
    pub unsupported_facet: bool,
}

impl Coverage {
    pub fn validate(&self) -> Result<()> {
        git::oid(&self.source_commit)?;
        let receipt = &self.producer_receipt_id;
        if receipt.len() != 64
            || !receipt
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return refuse("coverage requires a lowercase SHA-256 producer receipt ID");
        }
        for field in [&self.selection, &self.configuration] {
            if field.is_empty() || field.len() > 4096 || field.contains('\0') {
                return refuse("invalid coverage scope");
            }
        }
        if self.limitations.len() > 256
            || self
                .limitations
                .iter()
                .any(|s| s.is_empty() || s.len() > 4096)
        {
            return refuse("invalid coverage limitation");
        }
        if let Some(total) = self.declared_total {
            let accounted = self
                .accounting
                .examined
                .checked_add(self.accounting.unsupported)
                .and_then(|n| n.checked_add(self.accounting.excluded))
                .ok_or("coverage count overflow")?;
            if accounted > total {
                return refuse("observations exceed declared finite scope");
            }
        }
        Ok(())
    }

    pub fn status(&self) -> Result<Status> {
        self.validate()?;
        if self.unsupported_facet {
            return Ok(Status::Unsupported);
        }
        let a = self.accounting;
        let Some(total) = self.declared_total else {
            return Ok(
                if a.examined == 0 && a.unsupported == 0 && a.excluded == 0 && a.unresolved == 0 {
                    Status::Unknown
                } else {
                    Status::Partial
                },
            );
        };
        if self.configuration_resolved
            && !a.truncated
            && a.unsupported == 0
            && a.excluded == 0
            && a.unresolved == 0
            && a.examined == total
        {
            Ok(Status::CompleteDeclaredScope)
        } else {
            Ok(Status::Partial)
        }
    }

    /// No absence inference can be made from mere accounting; an independent,
    /// facet-specific closed-world oracle is required by later verification.
    pub fn absence_is_proven(&self) -> bool {
        false
    }

    pub fn report(&self) -> Result<Value> {
        let status = self.status()?;
        let a = self.accounting;
        let body = obj(&[
            ("schema", Value::string("excession.evidence-coverage.v1")),
            (
                "producer_receipt_id",
                Value::string(&self.producer_receipt_id),
            ),
            ("source_commit", Value::string(&self.source_commit)),
            ("facet", Value::string(self.facet.as_str())),
            (
                "scope",
                obj(&[
                    ("selection", Value::string(&self.selection)),
                    ("configuration", Value::string(&self.configuration)),
                    (
                        "configuration_resolved",
                        Value::Bool(self.configuration_resolved),
                    ),
                    (
                        "universe",
                        Value::string(if self.declared_total.is_some() {
                            "FINITE_DECLARED"
                        } else {
                            "UNBOUNDED_OR_UNKNOWN"
                        }),
                    ),
                    (
                        "declared_total",
                        self.declared_total.map_or(Value::Null, Value::number),
                    ),
                ]),
            ),
            ("status", Value::string(status.as_str())),
            ("examined", Value::number(a.examined)),
            ("unsupported", Value::number(a.unsupported)),
            ("unresolved", Value::number(a.unresolved)),
            ("excluded", Value::number(a.excluded)),
            ("truncated", Value::Bool(a.truncated)),
            (
                "limitations",
                Value::array(self.limitations.iter().map(Value::string)),
            ),
            (
                "negative_claim_scope",
                Value::string("INDEPENDENT_ORACLE_REQUIRED"),
            ),
        ]);
        let mut record = body.entries()?.clone();
        record.insert("report_id".to_owned(), Value::string(body.sha256()));
        Ok(Value::Object(record))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(facet: Facet) -> Coverage {
        Coverage {
            source_commit: "a".repeat(40),
            producer_receipt_id: "b".repeat(64),
            facet,
            selection: "src/**".into(),
            configuration: "rust-analyzer:default".into(),
            configuration_resolved: true,
            declared_total: Some(3),
            accounting: Accounting {
                examined: 3,
                unsupported: 0,
                excluded: 0,
                unresolved: 0,
                truncated: false,
            },
            limitations: Vec::new(),
            unsupported_facet: false,
        }
    }

    #[test]
    fn finite_declared_scope_is_not_behavioral_proof() {
        for facet in [Facet::GitSources, Facet::ScipReferences, Facet::Calls] {
            let coverage = sample(facet);
            assert_eq!(coverage.status().unwrap(), Status::CompleteDeclaredScope);
            assert!(!coverage.absence_is_proven());
        }
    }

    #[test]
    fn empty_or_unknown_index_does_not_certify_absence() {
        let mut coverage = sample(Facet::ScipReferences);
        coverage.declared_total = None;
        coverage.accounting.examined = 0;
        assert_eq!(coverage.status().unwrap(), Status::Unknown);
        assert!(!coverage.absence_is_proven());
        coverage.accounting.examined = 1;
        assert_eq!(coverage.status().unwrap(), Status::Partial);
        assert!(!coverage.absence_is_proven());
    }

    #[test]
    fn excluded_unsupported_unresolved_and_truncated_scopes_are_partial() {
        let mut coverage = sample(Facet::Calls);
        coverage.accounting.examined = 2;
        coverage.accounting.unsupported = 1;
        assert_eq!(coverage.status().unwrap(), Status::Partial);
        coverage.accounting.unsupported = 0;
        coverage.accounting.excluded = 1;
        assert_eq!(coverage.status().unwrap(), Status::Partial);
        coverage.accounting.excluded = 0;
        coverage.accounting.unresolved = 1;
        assert_eq!(coverage.status().unwrap(), Status::Partial);
        coverage.accounting.unresolved = 0;
        coverage.accounting.truncated = true;
        assert_eq!(coverage.status().unwrap(), Status::Partial);
    }

    #[test]
    fn incomplete_config_and_forged_counts_refuse_completion() {
        let mut coverage = sample(Facet::BuildInputs);
        coverage.configuration_resolved = false;
        assert_eq!(coverage.status().unwrap(), Status::Partial);
        coverage.configuration_resolved = true;
        coverage.accounting.examined = 4;
        assert!(coverage.status().is_err());
        coverage.accounting.examined = 3;
        coverage.producer_receipt_id = "INVALID".into();
        assert!(coverage.status().is_err());
    }

    #[test]
    fn deterministic_report_preserves_facet_and_explicit_limits() {
        let coverage = sample(Facet::GitSources);
        let report = coverage.report().unwrap();
        assert_eq!(report, coverage.report().unwrap());
        assert_eq!(report.get("facet").unwrap().str().unwrap(), "GIT_SOURCES");
        assert_eq!(
            report.get("status").unwrap().str().unwrap(),
            "COMPLETE_DECLARED_SCOPE"
        );
        assert_eq!(
            report.get("negative_claim_scope").unwrap().str().unwrap(),
            "INDEPENDENT_ORACLE_REQUIRED"
        );
    }
}
