//! Canonical projection plan and Git-source authentication.
//! All consumers share one identity, validation, and refusal contract.
use crate::json::{self, Value, obj};
use crate::{Result, digest, git, refuse};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

const FORMAT: &str = "excession.repository-view.v1";
pub(super) const MAX_ENTRIES: usize = 5_000;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Entry {
    pub(super) virtual_path: String,
    pub(super) source_path: String,
    pub(super) blob: String,
    pub(super) mode: String,
    pub(super) role: String,
    pub(super) detail: String,
}

impl Entry {
    pub(super) fn to_json(&self) -> Value {
        obj(&[
            ("virtual_path", Value::string(&self.virtual_path)),
            ("source_path", Value::string(&self.source_path)),
            ("blob", Value::string(&self.blob)),
            ("mode", Value::string(&self.mode)),
            ("role", Value::string(&self.role)),
            ("detail", Value::string(&self.detail)),
        ])
    }
    fn from_json(v: &Value) -> Result<Self> {
        keys(
            v,
            &[
                "virtual_path",
                "source_path",
                "blob",
                "mode",
                "role",
                "detail",
            ],
        )?;
        Ok(Self {
            virtual_path: v.get("virtual_path")?.str()?.to_owned(),
            source_path: v.get("source_path")?.str()?.to_owned(),
            blob: v.get("blob")?.str()?.to_owned(),
            mode: v.get("mode")?.str()?.to_owned(),
            role: v.get("role")?.str()?.to_owned(),
            detail: v.get("detail")?.str()?.to_owned(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub(super) kind: String,
    pub(super) commit: String,
    pub(super) tree: String,
    pub(super) focus: String,
    pub(super) entries: Vec<Entry>,
}

fn keys(v: &Value, expected: &[&str]) -> Result<()> {
    let m = v.entries()?;
    if m.len() != expected.len() || expected.iter().any(|key| !m.contains_key(*key)) {
        return refuse("unexpected or missing view manifest field");
    }
    Ok(())
}

impl Plan {
    pub(super) fn payload(&self) -> Value {
        obj(&[
            ("schema", Value::string(FORMAT)),
            ("kind", Value::string(&self.kind)),
            ("source_commit", Value::string(&self.commit)),
            ("source_tree", Value::string(&self.tree)),
            ("focus", Value::string(&self.focus)),
            (
                "entries",
                Value::array(self.entries.iter().map(Entry::to_json)),
            ),
        ])
    }
    /// Canonical payload bytes in the established BTreeMap key order.
    ///
    /// This is an alternate serialization of exactly the six fields produced
    /// by payload(). It avoids constructing nested JSON Value objects solely
    /// to obtain a view ID. The independent Value serializer remains the
    /// correctness oracle in tests and the source of public manifests.
    pub(super) fn canonical_payload(&self) -> String {
        let mut output = String::with_capacity(256 + self.entries.len() * 192);
        output.push_str(r#"{"entries":["#);
        for (i, entry) in self.entries.iter().enumerate() {
            if i != 0 {
                output.push(',');
            }
            output.push_str(r#"{"blob":"#);
            json::write_quoted(&entry.blob, &mut output);
            output.push_str(r#","detail":"#);
            json::write_quoted(&entry.detail, &mut output);
            output.push_str(r#","mode":"#);
            json::write_quoted(&entry.mode, &mut output);
            output.push_str(r#","role":"#);
            json::write_quoted(&entry.role, &mut output);
            output.push_str(r#","source_path":"#);
            json::write_quoted(&entry.source_path, &mut output);
            output.push_str(r#","virtual_path":"#);
            json::write_quoted(&entry.virtual_path, &mut output);
            output.push('}');
        }
        output.push_str(r#"],"focus":"#);
        json::write_quoted(&self.focus, &mut output);
        output.push_str(r#","kind":"#);
        json::write_quoted(&self.kind, &mut output);
        output.push_str(r#","schema":"#);
        json::write_quoted(FORMAT, &mut output);
        output.push_str(r#","source_commit":"#);
        json::write_quoted(&self.commit, &mut output);
        output.push_str(r#","source_tree":"#);
        json::write_quoted(&self.tree, &mut output);
        output.push('}');
        output
    }

    pub(super) fn view_id(&self) -> String {
        digest::sha256(self.canonical_payload().as_bytes())
    }

    /// Serialize the public manifest without materializing the nested JSON
    /// Value tree. The source-authenticated Plan and canonical payload order
    /// are unchanged. Append the final sorted view_id field after hashing
    /// exactly the original six-field payload.
    pub(super) fn canonical_manifest(&self) -> String {
        let mut output = self.canonical_payload();
        let id = digest::sha256(output.as_bytes());
        assert_eq!(output.pop(), Some('}'), "canonical payload ends in object");
        output.push_str(r#","view_id":"#);
        json::write_quoted(&id, &mut output);
        output.push('}');
        output
    }

    pub fn manifest(&self) -> Value {
        let view_id = self.view_id();
        let payload = self.payload();
        let Value::Object(mut fields) = payload else {
            unreachable!("projection payload is always a JSON object")
        };
        fields.insert("view_id".to_owned(), Value::string(view_id));
        Value::Object(fields)
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > 5 * 1024 * 1024 {
            return refuse("view manifest exceeds 5 MiB");
        }
        let v = json::parse(bytes)?;
        keys(
            &v,
            &[
                "schema",
                "kind",
                "source_commit",
                "source_tree",
                "focus",
                "entries",
                "view_id",
            ],
        )?;
        if v.get("schema")?.str()? != FORMAT {
            return refuse("unknown view manifest format");
        }
        let entries = v
            .get("entries")?
            .array_ref()?
            .iter()
            .map(Entry::from_json)
            .collect::<Result<Vec<_>>>()?;
        let plan = Self {
            kind: v.get("kind")?.str()?.to_owned(),
            commit: git::oid(v.get("source_commit")?.str()?)?,
            tree: git::oid(v.get("source_tree")?.str()?)?,
            focus: v.get("focus")?.str()?.to_owned(),
            entries,
        };
        plan.check()?;
        if plan.view_id() != v.get("view_id")?.str()? {
            return refuse("view ID does not authenticate canonical manifest");
        }
        Ok(plan)
    }
    pub(super) fn check(&self) -> Result<()> {
        git::oid(&self.commit)?;
        git::oid(&self.tree)?;
        if !matches!(
            self.kind.as_str(),
            "dependencies" | "security-flow" | "composed"
        ) || self.focus.is_empty()
        {
            return refuse("unsupported projection kind or missing focus");
        }
        if self.entries.is_empty() || self.entries.len() > MAX_ENTRIES {
            return refuse("view requires 1..5000 entries");
        }
        let mut unique = BTreeSet::new();
        for e in &self.entries {
            git::validate_path(&e.virtual_path)?;
            if !e.virtual_path.starts_with("views/") {
                return refuse("virtual paths must live below views/");
            }
            git::validate_path(&e.source_path)?;
            git::oid(&e.blob)?;
            if !matches!(e.mode.as_str(), "100644" | "100755") {
                return refuse("non-regular source is not a view entry");
            }
            if e.role.is_empty() || e.role.len() > 80 || e.detail.len() > 512 {
                return refuse("invalid view role or detail");
            }
            if !unique.insert(e.virtual_path.as_str()) {
                return refuse("colliding virtual paths");
            }
        }
        if self
            .entries
            .windows(2)
            .any(|pair| pair[0].virtual_path >= pair[1].virtual_path)
        {
            return refuse("view entries are not canonically sorted");
        }
        // A virtual file must never simultaneously be a parent directory.
        // This also prevents ambiguous navigation and failed materialization.
        for e in &self.entries {
            let mut ancestor = e.virtual_path.as_str();
            while let Some((parent, _)) = ancestor.rsplit_once('/') {
                if unique.contains(parent) {
                    return refuse("virtual file/directory path collision");
                }
                ancestor = parent;
            }
        }
        Ok(())
    }
}

pub(super) fn source_table(
    repo: &Path,
    commit: &str,
    tree: &str,
) -> Result<BTreeMap<String, git::Entry>> {
    git::verify_repo(repo)?;
    if git::rev(repo, &format!("{commit}^{{tree}}"))? != tree {
        return refuse("source commit and root tree disagree");
    }
    let mut table = BTreeMap::new();
    for entry in git::entries(repo, commit)? {
        if entry.kind == "blob" {
            table.insert(entry.path.clone(), entry);
        }
    }
    Ok(table)
}

/// Check all entries against a source table independently authenticated for
/// this exact commit/tree. Reusing an enumeration never reuses a verdict.
pub(super) fn authenticate_from_table(
    plan: &Plan,
    actual: &BTreeMap<String, git::Entry>,
) -> Result<()> {
    plan.check()?;
    check_source_bindings(plan, actual)
}

fn check_source_bindings(plan: &Plan, actual: &BTreeMap<String, git::Entry>) -> Result<()> {
    for e in &plan.entries {
        let source = actual
            .get(&e.source_path)
            .ok_or("view references an absent source")?;
        if source.oid != e.blob || source.mode != e.mode {
            return refuse(format!("view source identity mismatch: {}", e.source_path));
        }
    }
    Ok(())
}

pub(super) fn authenticate(plan: &Plan, repo: &Path) -> Result<()> {
    plan.check()?;
    let actual = source_table(repo, &plan.commit, &plan.tree)?;
    check_source_bindings(plan, &actual)
}

pub(super) fn make_plan(
    kind: &str,
    focus: &str,
    commit: String,
    tree: String,
    mut entries: Vec<Entry>,
) -> Result<Plan> {
    entries.sort_by(|a, b| a.virtual_path.cmp(&b.virtual_path));
    let plan = Plan {
        kind: kind.into(),
        commit,
        tree,
        focus: focus.into(),
        entries,
    };
    plan.check()?;
    Ok(plan)
}

/// Virtual views are immutable selections over the same source tree. The repository
/// remains authoritative; neither composition nor navigation copies source bytes.
pub(super) fn checked_plan(repo: &Path, manifest: &Path) -> Result<Plan> {
    let plan = read_plan(manifest)?;
    authenticate(&plan, repo)?;
    Ok(plan)
}

pub(super) fn read_plan(path: &Path) -> Result<Plan> {
    Plan::from_bytes(&fs::read(path).map_err(|e| e.to_string())?)
}
