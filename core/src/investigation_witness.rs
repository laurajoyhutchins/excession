//! Line-precise, Git-authenticated source witnesses for proposed control, data
//! and state behavior. The source text is independently re-read during ledger
//! replay. The interpretation is still an operator assertion, not runtime fact.
use crate::evidence::identity::SourceRef;
use crate::json::{Value, obj};
use crate::{Result, digest, git, investigation, refuse};
use std::path::Path;

const MAX_SPAN_LINES: usize = 32;
const MAX_SPAN_BYTES: usize = 8192;
const MAX_INTERPRETATION: usize = 8192;
const CHUNK_BYTES: usize = 64 * 1024;

fn kind(name: &str) -> Result<()> {
    if !matches!(
        name,
        "CONTROL_CONDITION" | "DATA_TRANSFER" | "STATE_MUTATION"
    ) {
        return refuse(
            "behavior witness kind must be CONTROL_CONDITION, DATA_TRANSFER, or STATE_MUTATION",
        );
    }
    Ok(())
}

fn interpretation(value: &str) -> Result<()> {
    if value.trim().is_empty() || value.len() > MAX_INTERPRETATION || value.contains('\0') {
        return refuse("behavior witness interpretation must be nonempty and bounded");
    }
    Ok(())
}

fn line_range(first: usize, last: usize) -> Result<()> {
    if first == 0 || last < first || last - first >= MAX_SPAN_LINES {
        return refuse("behavior witness requires 1..=32 ordered source lines");
    }
    Ok(())
}

/// Stream the *entire* Git blob through bounded text windows, verifying its
/// object ID on completion, but retain only the selected short source span.
/// A long excluded file never needs to fit in memory. Oversized target spans
/// refuse without pretending the file is absent.
fn exact_span(repo: &Path, source: &SourceRef, first: usize, last: usize) -> Result<String> {
    line_range(first, last)?;
    let mut reader = git::BlobReader::open(repo)?;
    let mut selected = Vec::new();
    let mut seen_last = false;
    reader.stream_text_chunks_verified(&source.blob, CHUNK_BYTES, |_, start, chunk| {
        let mut current = start;
        for piece in chunk.split_inclusive('\n') {
            if piece.is_empty() {
                continue;
            }
            if (first..=last).contains(&current) {
                let new_size = selected
                    .len()
                    .checked_add(piece.len())
                    .ok_or("behavior witness source span length overflow")?;
                if new_size > MAX_SPAN_BYTES {
                    return refuse("behavior witness source span exceeds 8192 bytes");
                }
                selected.extend_from_slice(piece.as_bytes());
                if current == last {
                    seen_last = true;
                }
            }
            if piece.ends_with('\n') {
                current += 1;
            }
        }
        Ok(())
    })?;
    reader.finish()?;
    if selected.is_empty() || !seen_last {
        return refuse("behavior witness source lines absent or empty");
    }
    String::from_utf8(selected).map_err(|_| "source span was not valid UTF-8".to_owned())
}

fn prior_claim<'a>(prior: &'a [Value], claim_id: &str) -> Result<&'a Value> {
    if claim_id.len() != 64
        || !claim_id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return refuse("behavior witness claim reference is not an exact event identity");
    }
    prior
        .iter()
        .find(|event| {
            event.get("event_id").and_then(Value::str).ok() == Some(claim_id)
                && event.get("kind").and_then(Value::str).ok() == Some("CLAIM")
        })
        .ok_or_else(|| "behavior witness must refer to an earlier claim".to_owned())
}

pub(crate) fn validate_event(
    repo: &Path,
    commit: &str,
    tree: &str,
    payload: &Value,
    prior: &[Value],
) -> Result<()> {
    if payload.entries()?.len() != 10 {
        return refuse("behavior witness payload has unexpected fields");
    }
    let claim = prior_claim(prior, payload.get("claim_event_id")?.str()?)?;
    let source = investigation::source_from_value(payload.get("source")?)?;
    source.verify_git(repo)?;
    if source.commit != commit
        || source.tree != tree
        || payload.get("source")? != claim.get("payload")?.get("source")?
    {
        return refuse("behavior witness must belong to its source-grounded claim");
    }
    kind(payload.get("kind")?.str()?)?;
    interpretation(payload.get("interpretation")?.str()?)?;
    if payload.get("assessment")?.str()? != "SOURCE_TEXT_VERIFIED_INTERPRETATION_UNVERIFIED"
        || payload.get("runtime_verified")?.bool()?
    {
        return refuse("behavior witness cannot assert semantic or runtime verification");
    }
    let first = usize::try_from(payload.get("first_line")?.integer()?)
        .map_err(|_| "behavior witness first line must be positive")?;
    let last = usize::try_from(payload.get("last_line")?.integer()?)
        .map_err(|_| "behavior witness last line must be positive")?;
    let text = exact_span(repo, &source, first, last)?;
    if payload.get("source_text")?.str()? != text
        || payload.get("source_text_sha256")?.str()? != digest::sha256(text.as_bytes())
    {
        return refuse("behavior witness source span differs from pinned Git text");
    }
    Ok(())
}

pub struct Witness<'a> {
    pub id: &'a str,
    pub expected_revision: usize,
    pub claim_event_id: &'a str,
    pub first_line: usize,
    pub last_line: usize,
    pub kind: &'a str,
    pub interpretation: &'a str,
}

pub fn record(db: &crate::sqlite::Db, repo: &Path, request: Witness<'_>) -> Result<Value> {
    kind(request.kind)?;
    interpretation(request.interpretation)?;
    line_range(request.first_line, request.last_line)?;
    let history = investigation::audit(db, repo, request.id)?;
    if history.get("revision")?.integer()? != request.expected_revision as i64 {
        return refuse("stale investigation expected revision");
    }
    let claim = prior_claim(history.get("events")?.array_ref()?, request.claim_event_id)?;
    let source = investigation::source_from_value(claim.get("payload")?.get("source")?)?;
    let text = exact_span(repo, &source, request.first_line, request.last_line)?;
    investigation::append(
        db,
        repo,
        request.id,
        request.expected_revision,
        "WITNESS",
        obj(&[
            ("claim_event_id", Value::string(request.claim_event_id)),
            ("source", investigation::source_value(&source)),
            ("first_line", Value::number(request.first_line)),
            ("last_line", Value::number(request.last_line)),
            ("source_text", Value::string(&text)),
            (
                "source_text_sha256",
                Value::string(digest::sha256(text.as_bytes())),
            ),
            ("kind", Value::string(request.kind)),
            ("interpretation", Value::string(request.interpretation)),
            (
                "assessment",
                Value::string("SOURCE_TEXT_VERIFIED_INTERPRETATION_UNVERIFIED"),
            ),
            ("runtime_verified", Value::Bool(false)),
        ]),
        None,
    )
}
