//! Historical source relationship and optional fresh target execution.
use super::PrivateDir;
use super::execution::execute;
use super::filesystem::materialize;
use super::planning::{plan, validate};
use crate::json::{Value, obj};
use crate::{Result, refuse, slice_evidence};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub fn carry(repo: &Path, manifest: &Value, receipt: &Value, revision: &str) -> Result<Value> {
    let (old_commit, old_sources) = validate(repo, manifest)?;
    let view = manifest.get("view_id")?.str()?;
    let entry = manifest.get("entry")?.str()?;
    let receipt_id = slice_evidence::validated_receipt(receipt, view, &old_commit, entry)?;
    if receipt.get("status")?.str()? != "OBSERVED_PASS" {
        return refuse("only passing source observations may be carried");
    }
    let assets = manifest
        .get("assets")?
        .array_ref()?
        .iter()
        .map(|v| v.str().map(str::to_owned))
        .collect::<Result<Vec<_>>>()?;
    let target = plan(repo, revision, entry, &assets, true)?;
    let (_, target_sources) = validate(repo, &target)?;
    let old = old_sources
        .iter()
        .map(|v| (&v.path, &v.blob))
        .collect::<BTreeMap<_, _>>();
    let new = target_sources
        .iter()
        .map(|v| (&v.path, &v.blob))
        .collect::<BTreeMap<_, _>>();
    let changed = old
        .keys()
        .chain(new.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|key| old.get(*key) != new.get(*key))
        .map(|key| Value::string(key.as_str()))
        .collect::<Vec<_>>();
    let unchanged = changed.is_empty();
    Ok(obj(&[
        ("schema", Value::string("excession.evidence-carry.v1")),
        ("source_commit", Value::string(old_commit)),
        ("target_commit", target.get("commit")?.clone()),
        ("source_view_id", manifest.get("view_id")?.clone()),
        ("target_view_id", target.get("view_id")?.clone()),
        ("previous_receipt_id", Value::string(receipt_id)),
        ("changed_closure_paths", Value::array(changed)),
        (
            "decision",
            Value::string(if unchanged {
                "SOURCE_MATCH_ONLY"
            } else {
                "REVERIFY_REQUIRED"
            }),
        ),
        ("target_behavior_verified", Value::Bool(false)),
        (
            "reason",
            Value::string(if unchanged {
                "Source closure unchanged; runtime/environment effects outside the slice remain unverified"
            } else {
                "Source closure or import resolution changed; fresh target tests required"
            }),
        ),
    ]))
}

pub fn replay(
    repo: &Path,
    manifest: &Value,
    receipt: &Value,
    revision: &str,
    isolated: bool,
    trusted: bool,
) -> Result<Value> {
    let mut result = carry(repo, manifest, receipt, revision)?;
    if !trusted {
        return refuse("target execution requires explicit --trusted-local true");
    }
    let entry = manifest.get("entry")?.str()?;
    let assets = manifest
        .get("assets")?
        .array_ref()?
        .iter()
        .map(|v| v.str().map(str::to_owned))
        .collect::<Result<Vec<_>>>()?;
    let target = plan(repo, revision, entry, &assets, true)?;
    let tmp = PrivateDir::new(&std::env::temp_dir(), "excession-replay")?;
    let location = tmp.path().join("slice");
    materialize(repo, &target, &location)?;
    let observation = execute(repo, &target, &location, 10, isolated, true)?;
    result
        .entries_mut()?
        .insert("fresh_target_receipt".into(), observation);
    Ok(result)
}
