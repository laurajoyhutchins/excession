//! A Rust-native incremental-value gate over a single pinned SCIP index.
//! Source pair overlap is not independently verified architectural meaning.
use crate::concepts;
use crate::concepts_compile;
use crate::concepts_scip;
use crate::json::{Value, obj};
use crate::scip_refactor;
use crate::{Result, refuse};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

type Source = (String, String);
type Pair = (Source, Source);

fn identity(node: &Value) -> Result<Source> {
    let path = node.get("path")?.str()?;
    crate::git::validate_path(path)?;
    let blob = node.get("blob")?.str()?;
    if blob.len() != 40
        || !blob
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    {
        return refuse("source identity requires a lowercase 40-digit Git blob");
    }
    Ok((path.to_owned(), blob.to_owned()))
}

fn pair(a: Source, b: Source) -> Result<Pair> {
    if a == b {
        return refuse("a concept candidate must bind two distinct sources");
    }
    Ok(if a < b { (a, b) } else { (b, a) })
}

fn base_pairs(report: &Value) -> Result<BTreeSet<Pair>> {
    if report.get("schema")?.str()? != "excession.scip-refactor.v1"
        || report.get("status")?.str()? != "UNVERIFIED_SCIP_RELATIONS"
        || concepts::content_id(report, "report_id")? != report.get("report_id")?.str()?
    {
        return refuse("value gate baseline is not a canonical native SCIP report");
    }
    let mut result = BTreeSet::new();
    for candidate in report.get("candidates")?.array_ref()? {
        if candidate.get("kind")?.str()? != "reciprocal_symbol_references" {
            continue;
        }
        if candidate.get("status")?.str()? != "UNVERIFIED_REFACTOR_HYPOTHESIS" {
            return refuse("native reciprocal reference was promoted to verified");
        }
        let sources = candidate.get("sources")?.array_ref()?;
        if sources.len() != 2 {
            return refuse("native reciprocal reference requires exactly two sources");
        }
        if !result.insert(pair(identity(&sources[0])?, identity(&sources[1])?)?) {
            return refuse("duplicate native reciprocal file pair");
        }
    }
    Ok(result)
}

fn semantic_pairs(model: &Value) -> Result<BTreeSet<Pair>> {
    concepts::validate_model(model)?;
    let subjects = model
        .get("subjects")?
        .array_ref()?
        .iter()
        .map(|v| Ok((v.get("subject_id")?.str()?.to_owned(), v)))
        .collect::<Result<BTreeMap<_, _>>>()?;
    let claims = model
        .get("claims")?
        .array_ref()?
        .iter()
        .map(|v| Ok((v.get("claim_id")?.str()?.to_owned(), v)))
        .collect::<Result<BTreeMap<_, _>>>()?;
    let mut result = BTreeSet::new();
    for c in model.get("concepts")?.array_ref()? {
        if c.get("review_status")?.str()? != "UNREVIEWED" {
            return refuse("source-only proposal cannot claim review certification");
        }
        let memberships = c.get("membership_claims")?.array_ref()?;
        if memberships.len() != 2 {
            return refuse("source-pair concept has wrong number of memberships");
        }
        let mut sources = Vec::new();
        for id in memberships {
            let claim = claims.get(id.str()?).ok_or("missing membership claim")?;
            if claim.get("predicate")?.str()? != "REALIZES_CONCEPT"
                || claim.get("assessment")?.str()? != "PROPOSED"
                || claim.get("behavioral_verification")?.str()? != "UNTESTED"
                || claim.get("concept_id")? != c.get("concept_id")?
            {
                return refuse("semantic membership is not an unverified proposal");
            }
            let ids = claim.get("subjects")?.array_ref()?;
            if ids.len() != 1 {
                return refuse("membership must bind exactly one native source");
            }
            let node = subjects
                .get(ids[0].str()?)
                .ok_or("missing source identity")?;
            sources.push(identity(node)?);
        }
        if !result.insert(pair(sources[0].clone(), sources[1].clone())?) {
            return refuse("two concept candidates duplicate one native source pair");
        }
    }
    Ok(result)
}

/// A deliberately narrow comparison, not an independent concept-quality oracle.
pub fn compare(baseline: &Value, batch: &Value, model: &Value) -> Result<Value> {
    if concepts::content_id(batch, "batch_id")? != batch.get("batch_id")?.str()?
        || batch.get("schema")?.str()? != "excession.concepts-observation-batch.v1"
        || batch.get("completeness")?.str()? != "PARTIAL_CANDIDATES_ONLY"
    {
        return refuse("value gate requires canonical partial observation batch");
    }
    if baseline.get("source_commit")? != batch.get("source_commit")?
        || baseline.get("source_tree")? != batch.get("source_tree")?
        || model.get("source_commit")? != batch.get("source_commit")?
        || model.get("source_tree")? != batch.get("source_tree")?
    {
        return refuse("baseline and concept revisions differ");
    }
    let inputs = batch.get("inputs")?.array_ref()?;
    if inputs.len() != 1
        || inputs[0].get("kind")?.str()? != "SCIP"
        || inputs[0].get("configuration_id")? != baseline.get("index_sha256")?
    {
        return refuse("baseline and concepts do not use the same native SCIP index");
    }
    let base = base_pairs(baseline)?;
    let proposed = semantic_pairs(model)?;
    let intersection = base.intersection(&proposed).count();
    let report = obj(&[
        (
            "schema",
            Value::string("excession.concepts-native-value.v1"),
        ),
        ("source_commit", baseline.get("source_commit")?.clone()),
        ("source_tree", baseline.get("source_tree")?.clone()),
        ("index_sha256", baseline.get("index_sha256")?.clone()),
        ("baseline_report_id", baseline.get("report_id")?.clone()),
        ("observation_batch_id", batch.get("batch_id")?.clone()),
        ("model_id", model.get("model_id")?.clone()),
        ("baseline_reciprocal_pairs", Value::number(base.len())),
        ("proposed_concept_pairs", Value::number(proposed.len())),
        ("unchanged_pairs", Value::number(intersection)),
        (
            "concept_only_pairs",
            Value::number(proposed.len() - intersection),
        ),
        (
            "baseline_only_pairs",
            Value::number(base.len() - intersection),
        ),
        (
            "independently_adjudicated_architectural_labels",
            Value::number(0),
        ),
        ("architectural_precision", Value::Null),
        ("architectural_recall", Value::Null),
        ("user_usefulness_gain", Value::Null),
        ("review_status", Value::string("UNJUDGED")),
        ("held_out_independent_oracle", Value::Bool(false)),
        ("decision", Value::string("DO_NOT_ENABLE_BY_DEFAULT")),
        ("baseline_bytes", Value::number(baseline.canonical().len())),
        (
            "semantic_model_bytes",
            Value::number(model.canonical().len()),
        ),
        (
            "normalized_observation_bytes",
            Value::number(batch.canonical().len()),
        ),
    ]);
    concepts::seal(report, "report_id")
}

pub fn compare_index(
    repo: &Path,
    revision: &str,
    index: &Path,
    prefix: Option<&str>,
) -> Result<Value> {
    let baseline = scip_refactor::suggest_with_prefix(repo, revision, index, prefix)?;
    let batch = concepts_scip::normalize_index(repo, revision, index, prefix)?;
    let model = concepts_compile::compile(&batch)?;
    compare(&baseline, &batch, &model)
}
