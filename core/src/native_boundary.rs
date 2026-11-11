//! Early boundary check between syntax candidates and compiler-produced *claims*.
//! Never promote a declaration to proof that a compiler ran, that all inputs
//! were observed, or that two executions are equivalent.
use crate::git;
use crate::json::{self, Value, obj};
use crate::{Result, preflight, refuse, syntax_resolution};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_DECLARED: usize = 512;
const MAX_ASSUMPTIONS: usize = 32;

fn read(path: &Path) -> Result<Value> {
    let len = std::fs::metadata(path)
        .map_err(|e| format!("native boundary stat: {e}"))?
        .len();
    if len > MAX_FILE_BYTES {
        return refuse("native boundary input exceeds 32 MiB");
    }
    let data = std::fs::read(path).map_err(|e| format!("native boundary read: {e}"))?;
    json::parse(&data)
}

fn exactly(value: &Value, expected: &[&str]) -> Result<()> {
    let fields = value.entries()?;
    if fields.len() != expected.len() || expected.iter().any(|k| !fields.contains_key(*k)) {
        return refuse("native declaration has unexpected or missing fields");
    }
    Ok(())
}

fn hex_sha256(value: &Value) -> Result<()> {
    let value = value.str()?;
    if value.len() != 64
        || !value
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    {
        return refuse("native toolchain fingerprint is not lowercase SHA-256");
    }
    Ok(())
}

fn nonempty(value: &Value, max: usize) -> Result<()> {
    let value = value.str()?;
    if value.is_empty() || value.len() > max {
        return refuse("native declaration has empty or oversized metadata");
    }
    Ok(())
}

/// Does not read Git, load grammars, execute compilers, or authenticate a worker.
pub fn declaration_preflight(value: &Value) -> Result<Value> {
    exactly(
        value,
        &[
            "schema",
            "coverage",
            "language",
            "subject",
            "inputs",
            "assumptions",
        ],
    )?;
    if value.get("schema")?.str()? != "excession.input-declaration.v1"
        || value.get("coverage")?.str()? != "DECLARED_INPUTS_ONLY"
    {
        return refuse("unsupported native input declaration contract");
    }
    let language = value.get("language")?.str()?;
    if !matches!(language, "go" | "rust" | "javascript" | "typescript") {
        return refuse("native declaration language unsupported");
    }
    let subject = value.get("subject")?.str()?;
    git::validate_path(subject)?;
    let inputs = value.get("inputs")?.array_ref()?;
    if inputs.is_empty() || inputs.len() > MAX_DECLARED {
        return refuse("native declaration requires 1 to 512 inputs");
    }
    let mut seen = BTreeSet::new();
    let mut subject_present = false;
    for entry in inputs {
        exactly(entry, &["path", "role"])?;
        let path = entry.get("path")?.str()?;
        git::validate_path(path)?;
        if !seen.insert(path.to_owned()) {
            return refuse("duplicate native input path");
        }
        let role = entry.get("role")?.str()?;
        if !matches!(role, "source" | "configuration") {
            return refuse("native input role unsupported");
        }
        if path == subject {
            if role != "source" {
                return refuse("native subject must be a source input");
            }
            subject_present = true;
        }
    }
    if !subject_present {
        return refuse("native subject missing from declared inputs");
    }
    let assumptions = value.get("assumptions")?.entries()?;
    if assumptions.len() > MAX_ASSUMPTIONS {
        return refuse("native assumptions exceed 32 keys");
    }
    for (key, val) in assumptions {
        if key.is_empty() || key.len() > 128 {
            return refuse("invalid native assumption key");
        }
        nonempty(val, 4096)?;
    }
    if assumptions
        .get("dependency-discovery")
        .ok_or("native resolver provenance missing")?
        .str()?
        != "excession-toolchain-native-v1"
    {
        return refuse("unsupported native resolver identity");
    }
    nonempty(
        assumptions
            .get("toolchain-version")
            .ok_or("missing toolchain version")?,
        180,
    )?;
    nonempty(assumptions.get("platform").ok_or("missing platform")?, 180)?;
    hex_sha256(
        assumptions
            .get("toolchain-sha256")
            .ok_or("missing toolchain SHA-256")?,
    )?;
    Ok(obj(&[
        ("status", Value::string("DECLARATION_SHAPE_ONLY")),
        ("declaration_id", Value::string(value.sha256())),
        ("input_count", Value::number(inputs.len())),
    ]))
}

pub fn preflight_file(path: &Path) -> Result<Value> {
    declaration_preflight(&read(path)?)
}

fn language_matches(declared: &str, observed: &str) -> bool {
    match declared {
        "go" => observed == "go",
        "rust" => observed == "rust",
        "javascript" => observed == "javascript",
        "typescript" => matches!(observed, "typescript" | "tsx" | "javascript"),
        _ => false,
    }
}

fn report(
    root: &Path,
    syntax_file: &Path,
    candidates_file: &Path,
    declaration_file: &Path,
) -> Result<Value> {
    // Check declared shapes and identities before invoking Tree-sitter.
    let declaration = read(declaration_file)?;
    declaration_preflight(&declaration)?;
    let syntax = read(syntax_file)?;
    preflight::syntax(&syntax)?;
    let candidates = read(candidates_file)?;
    preflight::resolution(&candidates)?;
    let revision = git::oid(syntax.get("source_commit")?.str()?)?;
    if candidates.get("source_commit")?.str()? != revision
        || candidates.get("source_tree")?.str()? != syntax.get("source_tree")?.str()?
        || candidates.get("syntax_report_id")?.str()? != syntax.get("report_id")?.str()?
    {
        return refuse(
            "native boundary: syntax and resolution reports have different source identities",
        );
    }
    let language = declaration.get("language")?.str()?;
    let subject = declaration.get("subject")?.str()?;
    let mut source_inputs = BTreeSet::new();
    let mut input_roles = BTreeMap::new();
    for entry in declaration.get("inputs")?.array_ref()? {
        let path = entry.get("path")?.str()?;
        let role = entry.get("role")?.str()?;
        input_roles.insert(path.to_owned(), role);
        if role == "source" {
            source_inputs.insert(path.to_owned());
        }
    }
    let mut scanned_paths = BTreeMap::new();
    for entry in syntax.get("sources")?.array_ref()? {
        scanned_paths.insert(
            entry.get("source_path")?.str()?,
            (entry.get("language")?.str()?, entry.get("status")?.str()?),
        );
    }
    if !matches!(scanned_paths.get(subject), Some((lang, "PARSED"))
        if language_matches(language, lang))
    {
        return refuse("native subject is absent or not successfully parsed in the syntax scope");
    }
    // Exact Git revision, regular tracked file and immutable blob identities.
    git::clean(root)?;
    if git::text(root, &["cat-file", "-t", &revision])? != "commit" {
        return refuse("native boundary source revision is not a commit");
    }
    if git::rev(root, &format!("{revision}^{{tree}}"))? != syntax.get("source_tree")?.str()? {
        return refuse("native boundary source tree mismatch");
    }
    let entries = git::entries(root, &revision)?;
    let by_path = entries
        .into_iter()
        .map(|entry| (entry.path.clone(), entry))
        .collect::<BTreeMap<_, _>>();
    let mut pinned_inputs = Vec::new();
    for (path, role) in &input_roles {
        let entry = by_path.get(path).ok_or_else(|| {
            format!("native input is not tracked at the selected Git revision: {path}")
        })?;
        if entry.kind != "blob" || !matches!(entry.mode.as_str(), "100644" | "100755") {
            return refuse("native input is not a regular Git blob");
        }
        pinned_inputs.push(obj(&[
            ("path", Value::string(path)),
            ("role", Value::string(*role)),
            ("blob", Value::string(&entry.oid)),
            ("mode", Value::string(&entry.mode)),
        ]));
    }
    // Costly replay is still compulsory: no source-independent graph may be promoted.
    syntax_resolution::audit(root, syntax_file, candidates_file)?;

    let mut matching = Vec::new();
    let mut static_only = Vec::new();
    let mut matched_targets = BTreeSet::new();
    for edge in candidates.get("candidate_edges")?.array_ref()? {
        let source = edge.get("source_path")?.str()?;
        let target = edge.get("target_path")?.str()?;
        let observed = edge.get("language")?.str()?;
        if !source_inputs.contains(source) || !language_matches(language, observed) {
            continue;
        }
        if source_inputs.contains(target) {
            matching.push(edge.clone());
            matched_targets.insert(target);
        } else {
            static_only.push(edge.clone());
        }
    }
    let mut native_only = Vec::new();
    let mut unscanned = Vec::new();
    for path in &source_inputs {
        if !scanned_paths.contains_key(path.as_str()) {
            unscanned.push(Value::string(path));
        }
        if path != subject && !matched_targets.contains(path.as_str()) {
            native_only.push(Value::string(path));
        }
    }
    let mut unresolved = Vec::new();
    for entry in candidates.get("unresolved_imports")?.array_ref()? {
        let path = entry.get("source_path")?.str()?;
        let observed = entry.get("language")?.str()?;
        if source_inputs.contains(path) && language_matches(language, observed) {
            unresolved.push(entry.clone());
        }
    }
    // Do not call any mismatch a proven missed dependency. Conditional compilation,
    // static over-approximation and compiler-owned input scopes legitimately differ.
    let status = if static_only.is_empty() {
        "NO_STATIC_NATIVE_DISAGREEMENT_OBSERVED"
    } else {
        "REVIEW_STATIC_NATIVE_SCOPE_DIFFERENCE"
    };
    let payload = obj(&[
        (
            "schema",
            Value::string("excession.native-syntax-boundary.v1"),
        ),
        ("source_commit", syntax.get("source_commit")?.clone()),
        ("source_tree", syntax.get("source_tree")?.clone()),
        ("syntax_report_id", syntax.get("report_id")?.clone()),
        ("resolution_id", candidates.get("resolution_id")?.clone()),
        ("declaration_id", Value::string(declaration.sha256())),
        ("language", declaration.get("language")?.clone()),
        ("subject", declaration.get("subject")?.clone()),
        ("coverage", Value::string("SCOPED_COMPARISON_ONLY")),
        ("provenance", Value::string("UNATTESTED_NATIVE_DECLARATION")),
        ("status", Value::string(status)),
        ("matching_edges", Value::array(matching)),
        ("static_only_edges", Value::array(static_only)),
        ("native_only_sources", Value::array(native_only)),
        ("unscanned_sources", Value::array(unscanned)),
        ("unresolved_imports", Value::array(unresolved)),
        ("pinned_inputs", Value::array(pinned_inputs)),
    ]);
    let mut fields = payload.entries()?.clone();
    fields.insert("boundary_id".into(), Value::string(payload.sha256()));
    Ok(Value::Object(fields))
}

pub fn compare(
    root: &Path,
    syntax_file: &Path,
    candidates_file: &Path,
    declaration_file: &Path,
) -> Result<Value> {
    report(root, syntax_file, candidates_file, declaration_file)
}

pub fn audit(
    root: &Path,
    syntax_file: &Path,
    candidates_file: &Path,
    declaration_file: &Path,
    saved: &Path,
) -> Result<Value> {
    let original = read(saved)?;
    let regenerated = report(root, syntax_file, candidates_file, declaration_file)?;
    if original.canonical() != regenerated.canonical() {
        return refuse("native boundary audit differs from reconstructed report");
    }
    Ok(obj(&[
        ("status", Value::string("DECLARATION_AND_GIT_REPLAY_MATCH")),
        ("boundary_id", regenerated.get("boundary_id")?.clone()),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Value {
        obj(&[
            ("schema", Value::string("excession.input-declaration.v1")),
            ("coverage", Value::string("DECLARED_INPUTS_ONLY")),
            ("language", Value::string("go")),
            ("subject", Value::string("src/main.go")),
            (
                "inputs",
                Value::array([
                    obj(&[
                        ("path", Value::string("src/main.go")),
                        ("role", Value::string("source")),
                    ]),
                    obj(&[
                        ("path", Value::string("go.mod")),
                        ("role", Value::string("configuration")),
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
                    ("toolchain-version", Value::string("go test")),
                    ("toolchain-sha256", Value::string("a".repeat(64))),
                    ("platform", Value::string("linux-x86_64")),
                ]),
            ),
        ])
    }

    #[test]
    fn declaration_has_no_attestation() {
        let claim = sample();
        let checked = declaration_preflight(&claim).unwrap();
        assert_eq!(
            checked.get("status").unwrap().str().unwrap(),
            "DECLARATION_SHAPE_ONLY"
        );
        assert_eq!(
            checked.get("declaration_id").unwrap().str().unwrap(),
            claim.sha256()
        );
    }

    #[test]
    fn missing_subject_or_invalid_fingerprint_is_refused() {
        let mut claim = sample();
        claim
            .entries_mut()
            .unwrap()
            .insert("subject".into(), Value::string("src/absent.go"));
        assert!(declaration_preflight(&claim).is_err());
        let mut claim = sample();
        let mut assumptions = claim.get("assumptions").unwrap().entries().unwrap().clone();
        assumptions.insert("toolchain-sha256".into(), Value::string("not-a-hash"));
        claim
            .entries_mut()
            .unwrap()
            .insert("assumptions".into(), Value::Object(assumptions));
        assert!(declaration_preflight(&claim).is_err());
    }

    #[test]
    fn duplicate_paths_and_wrong_roles_are_refused() {
        let mut claim = sample();
        let mut entries = claim.get("inputs").unwrap().array_ref().unwrap().to_vec();
        entries.push(entries[0].clone());
        claim
            .entries_mut()
            .unwrap()
            .insert("inputs".into(), Value::array(entries));
        assert!(declaration_preflight(&claim).is_err());
        let mut claim = sample();
        let mut entries = claim.get("inputs").unwrap().array_ref().unwrap().to_vec();
        entries[0] = obj(&[
            ("path", Value::string("src/main.go")),
            ("role", Value::string("configuration")),
        ]);
        claim
            .entries_mut()
            .unwrap()
            .insert("inputs".into(), Value::array(entries));
        assert!(declaration_preflight(&claim).is_err());
    }
}
