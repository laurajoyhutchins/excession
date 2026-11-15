//! Read-only attachment of existing build/REA evidence to reconstructed Git checkpoints.
//! Authentic receipt replay is not a behavioral oracle or an executed-build attestation.
use crate::evidence::{build, rea};
use crate::git_reconstruction;
use crate::json::{self, Value, obj};
use crate::sqlite::Db;
use crate::{Result, git, refuse};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

const MAX_MANIFEST: u64 = 1024 * 1024;
const MAX_BINDINGS: usize = 512;

fn exact_fields(value: &Value, fields: &[&str], context: &str) -> Result<()> {
    let entries = value.entries()?;
    if entries.len() != fields.len() || fields.iter().any(|key| !entries.contains_key(*key)) {
        return refuse(format!("{context} has missing or unexpected fields"));
    }
    Ok(())
}

fn read_bounded(path: &Path) -> Result<Value> {
    let meta = fs::metadata(path).map_err(|e| format!("evidence metadata: {e}"))?;
    if !meta.is_file() || meta.len() > MAX_MANIFEST {
        return refuse("Git reconstruction evidence must be a regular file <= 1 MiB");
    }
    let raw = fs::read(path).map_err(|e| format!("evidence read: {e}"))?;
    if raw.len() as u64 > MAX_MANIFEST {
        return refuse("Git reconstruction evidence changed or exceeded byte limit");
    }
    json::parse(&raw)
}

fn digest(value: &str) -> Result<&str> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return refuse("expected lowercase SHA-256 evidence identity");
    }
    Ok(value)
}

/// Reconcile externally stored, source-bound build/REA receipts with the
/// verified Git plan. This never executes source and writes no DB rows.
/// Each binding must contain *both* independently replayed existing receipts.
/// Missing bindings remain explicit rather than silently accepting a claim.
pub fn replay(db: &Db, repo: &Path, plan_path: &Path, evidence_path: &Path) -> Result<Value> {
    let structural = git_reconstruction::verify(repo, plan_path)?;
    let plan = read_bounded(plan_path)?;
    if plan.sha256() != structural.get("plan_sha256")?.str()? {
        return refuse("Git reconstruction plan changed between structural and evidence replay");
    }
    let evidence = read_bounded(evidence_path)?;
    exact_fields(&evidence, &["schema", "bindings"], "evidence manifest")?;
    if evidence.get("schema")?.str()? != "excession.git-reconstruction-evidence.v1" {
        return refuse("unsupported Git reconstruction evidence schema");
    }

    // A declaration is a candidate contract, *not* a verified behavior.
    let mut closures = BTreeMap::<String, BTreeSet<String>>::new();
    for contract in plan.get("contracts")?.array_ref()? {
        let id = contract.get("id")?.str()?.to_owned();
        let paths = contract
            .get("source_closure")?
            .array_ref()?
            .iter()
            .map(|v| Ok(v.str()?.to_owned()))
            .collect::<Result<BTreeSet<_>>>()?;
        if closures.insert(id, paths).is_some() {
            return refuse("duplicate contract declaration");
        }
    }

    // These checkpoint/contract pairs are derived from authenticated Git replay,
    // rather than trusted verbatim from the evidence manifest.
    let mut expected = BTreeSet::<(String, String, String)>::new();
    for history in structural.get("histories")?.array_ref()? {
        for stage in history.get("checkpoints")?.array_ref()? {
            let commit = stage.get("commit")?.str()?.to_owned();
            for claim in stage.get("introduced_contracts")?.array_ref()? {
                expected.insert((
                    "INTRODUCTION".to_owned(),
                    commit.clone(),
                    claim.str()?.to_owned(),
                ));
            }
        }
    }
    for node in structural.get("integration_nodes")?.array_ref()? {
        let commit = node.get("commit")?.str()?.to_owned();
        for claim in node.get("retained_contracts")?.array_ref()? {
            expected.insert((
                "INTEGRATION".to_owned(),
                commit.clone(),
                claim.str()?.to_owned(),
            ));
        }
    }
    if expected.len() > 2048 {
        return refuse("too many replay obligations");
    }
    let bindings = evidence.get("bindings")?.array_ref()?;
    if bindings.len() > MAX_BINDINGS {
        return refuse("too many evidence bindings");
    }
    let mut checked = BTreeSet::<(String, String, String)>::new();
    let mut reports = Vec::new();
    for binding in bindings {
        exact_fields(
            binding,
            &[
                "phase",
                "checkpoint",
                "contract",
                "artifact",
                "build_snapshot",
                "rea_bundle",
            ],
            "evidence binding",
        )?;
        let phase = binding.get("phase")?.str()?.to_owned();
        let checkpoint = binding.get("checkpoint")?.str()?.to_owned();
        let contract = binding.get("contract")?.str()?.to_owned();
        let key = (phase.clone(), checkpoint.clone(), contract.clone());
        if !expected.contains(&key) || !checked.insert(key) {
            return refuse("duplicate or unexpected contract/checkpoint evidence binding");
        }
        git::oid(&checkpoint)?;
        let build_id = digest(binding.get("build_snapshot")?.str()?)?;
        let bundle_id = digest(binding.get("rea_bundle")?.str()?)?;
        let artifact = Path::new(binding.get("artifact")?.str()?);
        if !artifact.is_absolute() {
            return refuse("evidence artifact path must be absolute");
        }
        let build_report = build::audit(db, repo, &checkpoint, artifact, build_id)?;
        if build_report.get("source_commit")?.str()? != checkpoint
            || !build_report.get("artifact_bytes_verified")?.bool()?
            || !build_report.get("git_inputs_verified")?.bool()?
            || build_report.get("build_execution_verified")?.bool()?
            || build_report.get("source_artifact_relation")?.str()? != "NOT_ESTABLISHED"
        {
            return refuse("build replay does not preserve the expected evidence boundary");
        }
        let source_inputs = build_report
            .get("build")?
            .get("source_inputs")?
            .array_ref()?
            .iter()
            .map(|input| Ok(input.get("path")?.str()?.to_owned()))
            .collect::<Result<BTreeSet<_>>>()?;
        let required = closures
            .get(&contract)
            .ok_or("missing reconstructed contract closure")?;
        if !required.is_subset(&source_inputs) {
            return refuse(format!(
                "build receipt omits source in contract closure: {contract}"
            ));
        }
        let link = rea::BuildLink {
            repo,
            revision: &checkpoint,
            snapshot: build_id,
        };
        let rea_report = rea::audit(db, artifact, bundle_id, Some(&link))?;
        if rea_report.get("build_snapshot_id")?.str()? != build_id
            || rea_report.get("artifact_sha256")?.str()?
                != build_report.get("artifact")?.get("sha256")?.str()?
            || rea_report.get("semantic_correctness")?.str()? != "NOT_ESTABLISHED"
            || rea_report.get("source_artifact_relation")?.str()? != "NOT_ESTABLISHED"
        {
            return refuse("REA replay differs from the exact linked build artifact");
        }
        reports.push(obj(&[
            ("phase", Value::string(phase)),
            ("checkpoint", Value::string(checkpoint)),
            ("contract", Value::string(contract)),
            ("build_snapshot", Value::string(build_id)),
            ("rea_bundle", Value::string(bundle_id)),
            (
                "artifact_sha256",
                build_report.get("artifact")?.get("sha256")?.clone(),
            ),
            ("source_closure_size", Value::number(required.len())),
            (
                "build_source_input_count",
                Value::number(source_inputs.len()),
            ),
            (
                "receipt_replay",
                Value::string("BUILD_AND_REA_BYTES_REPLAYED"),
            ),
            ("build_execution", Value::string("NOT_ESTABLISHED")),
            ("independent_oracle", Value::string("NOT_ESTABLISHED")),
            ("behavioral_preservation", Value::string("NOT_ESTABLISHED")),
        ]));
    }
    reports.sort_by_key(|a| a.canonical());
    let missing = expected
        .difference(&checked)
        .map(|(phase, checkpoint, contract)| {
            obj(&[
                ("phase", Value::string(phase)),
                ("checkpoint", Value::string(checkpoint)),
                ("contract", Value::string(contract)),
                (
                    "reason",
                    Value::string("NO_REPLAYED_BUILD_AND_REA_EVIDENCE"),
                ),
            ])
        })
        .collect::<Vec<_>>();
    let mut report = obj(&[
        (
            "schema",
            Value::string("excession.git-reconstruction-evidence-report.v1"),
        ),
        ("structural_report_id", structural.get("report_id")?.clone()),
        ("target_tree", structural.get("target_tree")?.clone()),
        ("manifest_sha256", Value::string(evidence.sha256())),
        ("obligation_count", Value::number(expected.len())),
        ("replayed_binding_count", Value::number(checked.len())),
        ("missing_binding_count", Value::number(missing.len())),
        ("bindings", Value::array(reports)),
        ("missing_bindings", Value::array(missing)),
        (
            "receipt_coverage",
            Value::string(if checked.len() == expected.len() {
                "COMPLETE_REPLAYED_BYTES_ONLY"
            } else {
                "INCOMPLETE"
            }),
        ),
        ("build_execution_attested", Value::Bool(false)),
        ("contract_oracles_verified", Value::Bool(false)),
        ("integration_behavior_verified", Value::Bool(false)),
        ("rea_semantic_correctness", Value::string("NOT_ESTABLISHED")),
    ]);
    let report_id = report.sha256();
    report
        .entries_mut()?
        .insert("report_id".into(), Value::string(report_id));
    Ok(report)
}
