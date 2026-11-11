//! Same-function semantic transaction composition using CPython AST statement slots.
//! Rust authenticates branch source trees, checks that edits commute syntactically,
//! creates a real two-parent Git commit, and reruns native joint obligations.
//! Syntactic commutation is NOT behavioral independence or program equivalence.
use crate::json::{self, Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, digest, git, refuse, transactions};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

type SqlRows = Vec<Vec<Option<String>>>;
type EvidenceRows = (SqlRows, SqlRows, SqlRows, SqlRows, SqlRows, SqlRows);

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS overlap_transaction(
  tx_id TEXT PRIMARY KEY,
  kind TEXT NOT NULL CHECK(kind IN ('left_edit','right_edit','composition')),
  target TEXT NOT NULL,
  base_commit TEXT NOT NULL,
  result_commit TEXT NOT NULL,
  result_tree TEXT NOT NULL,
  source_blob TEXT NOT NULL,
  evidence_status TEXT NOT NULL CHECK(evidence_status IN ('EXACT_AST_EDIT_AND_NATIVE_CHECK','FRESH_JOINT_VERIFICATION'))
);
CREATE TABLE IF NOT EXISTS overlap_dependency(
  prerequisite TEXT NOT NULL REFERENCES overlap_transaction(tx_id),
  dependent TEXT NOT NULL REFERENCES overlap_transaction(tx_id),
  kind TEXT NOT NULL CHECK(kind='requires'),
  PRIMARY KEY(prerequisite,dependent),
  CHECK(prerequisite<>dependent)
);
CREATE TABLE IF NOT EXISTS overlap_witness(
  tx_id TEXT NOT NULL REFERENCES overlap_transaction(tx_id),
  phase TEXT NOT NULL,
  command_hash TEXT NOT NULL,
  input_commit TEXT NOT NULL,
  observed_exit INTEGER NOT NULL CHECK(observed_exit=0),
  PRIMARY KEY(tx_id,phase,command_hash)
);
"#;

const OP_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS operation_effect(
 tx_id TEXT PRIMARY KEY REFERENCES overlap_transaction(tx_id),
 operation_kind TEXT NOT NULL CHECK(operation_kind IN ('codec.ascii_format','length.bytes_snapshot','length.drop_last')),
 statement_index INTEGER NOT NULL CHECK(statement_index BETWEEN 0 AND 500),
 precondition TEXT NOT NULL, effect TEXT NOT NULL,
 input_blob TEXT NOT NULL, output_blob TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS composition_obligation(
 tx_id TEXT NOT NULL REFERENCES overlap_transaction(tx_id),
 kind TEXT NOT NULL CHECK(kind IN ('branch_provenance','source_confluence','fresh_joint_checks')),
 evidence_digest TEXT NOT NULL,
 result TEXT NOT NULL CHECK(result='PASSED'),
 PRIMARY KEY(tx_id,kind)
);
"#;

const INFER_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS inferred_effect(
 tx_id TEXT PRIMARY KEY REFERENCES overlap_transaction(tx_id),
 role TEXT NOT NULL CHECK(role IN ('context_rewrite','subtree_rewrite')),
 statement_index INTEGER NOT NULL CHECK(statement_index BETWEEN 0 AND 500),
 base_statement_sha256 TEXT NOT NULL,
 branch_statement_sha256 TEXT NOT NULL,
 anchor_before_sha256 TEXT NOT NULL,
 anchor_after_sha256 TEXT NOT NULL,
 anchor_path TEXT NOT NULL,
 input_blob TEXT NOT NULL,
 output_blob TEXT NOT NULL
);
"#;

#[derive(Clone, Debug)]
pub struct Edit {
    pub index: usize,
    pub old: String,
    pub new: String,
}

#[derive(Clone)]
pub struct Spec {
    operations: Option<OperationPair>,
    inferred: Option<usize>,
    pub base: PathBuf,
    pub left: PathBuf,
    pub right: PathBuf,
    pub output: PathBuf,
    pub file: String,
    pub qualname: String,
    pub left_edit: Edit,
    pub right_edit: Edit,
    pub left_checks: Vec<Vec<String>>,
    pub right_checks: Vec<Vec<String>>,
    pub joint_checks: Vec<Vec<String>>,
}

#[derive(Clone, Debug)]
struct OperationPair {
    statement_index: usize,
    left: String,
    right: String,
}

fn operation_descriptor(kind: &str) -> Result<(&'static str, &'static str, &'static str)> {
    match kind {
        "codec.ascii_format" => Ok(("codec", "codec == percent", "codec := ascii_format")),
        "length.bytes_snapshot" => Ok(("length", "length == raw", "length := bytes_snapshot")),
        "length.drop_last" => Ok(("length", "length == raw", "length := drop_last")),
        _ => refuse("unsupported operation vocabulary"),
    }
}

impl Spec {
    pub fn is_inferred(&self) -> bool {
        self.inferred.is_some()
    }
    pub fn is_operation(&self) -> bool {
        self.operations.is_some()
    }
    fn composition_message(&self) -> &'static str {
        if self.is_inferred() {
            "Compose independently inferred AST context/subtree effects"
        } else if self.is_operation() {
            "Compose overlapping typed operations on one logical method"
        } else {
            "Compose independent edits to one logical method"
        }
    }
}

fn command(v: &Value) -> Result<Vec<String>> {
    let elements = v.array_ref()?;
    if elements.is_empty() || elements.len() > 16 {
        return refuse("native check command must contain 1 to 16 arguments");
    }
    let cmd = elements
        .iter()
        .map(|s| s.str().map(str::to_owned))
        .collect::<Result<Vec<_>>>()?;
    if !matches!(cmd[0].as_str(), "python" | "python3" | "node" | "go")
        || cmd.iter().any(|s| s.is_empty() || s.contains('\0'))
    {
        return refuse("native checks must use Python or Node argv, never a shell");
    }
    if cmd[0] == "go" && !matches!(cmd.get(1).map(String::as_str), Some("test" | "run")) {
        return refuse("Go checks restricted to go test and go run");
    }
    Ok(cmd)
}
fn suite(v: &Value, name: &str) -> Result<Vec<Vec<String>>> {
    let elements = v.get(name)?.array_ref()?;
    if elements.is_empty() || elements.len() > 16 {
        return refuse(format!(
            "{name}: at least one and at most 16 checks are required"
        ));
    }
    elements.iter().map(command).collect()
}
fn edit(v: &Value) -> Result<Edit> {
    let index = v.get("index")?.integer()?;
    if !(0..=500).contains(&index) {
        return refuse("invalid AST statement index");
    }
    let old = v.get("old")?.str()?.to_owned();
    let new = v.get("new")?.str()?.to_owned();
    if old.is_empty() || new.is_empty() || old.len() > 1000 || new.len() > 1000 || old == new {
        return refuse("invalid semantic edit contents");
    }
    Ok(Edit {
        index: index as usize,
        old,
        new,
    })
}
fn mode_is_inferred(v: &Value) -> bool {
    v.entries()
        .ok()
        .and_then(|e| e.get("mode"))
        .and_then(|m| m.str().ok())
        == Some("inferred_ast")
}
impl Spec {
    pub fn read(path: &Path) -> Result<Self> {
        let v = json::parse(&fs::read(path).map_err(|e| e.to_string())?)?;
        if !v.get("trusted_local")?.bool()? {
            return refuse("trusted_local must be explicitly true; untrusted agents unsupported");
        }
        let read_path = |name: &str| -> Result<PathBuf> {
            PathBuf::from(v.get(name)?.str()?)
                .canonicalize()
                .map_err(|e| format!("{name}: {e}"))
        };
        let output = PathBuf::from(v.get("output")?.str()?);
        if !output.is_absolute() {
            return refuse("output path must be absolute");
        }
        let file = v.get("file")?.str()?.to_owned();
        git::validate_path(&file)?;
        if !file.ends_with(".py")
            && !(mode_is_inferred(&v)
                && (file.ends_with(".js") || file.ends_with(".ts") || file.ends_with(".go")))
        {
            return refuse("only native AST source files supported in this transaction");
        }
        let qualname = v.get("qualname")?.str()?.to_owned();
        if !(v.entries()?.get("mode").and_then(|x| x.str().ok()) == Some("inferred_ast")
            && matches!(qualname.split('.').count(), 1 | 2)
            || qualname.split('.').count() == 2)
            || qualname.contains('/')
        {
            return refuse("expected exact function or Class.method target");
        }
        let mode = v.entries()?.get("mode").map(Value::str).transpose()?;
        let (left_edit, right_edit, operations, inferred) = if mode == Some("typed_operation") {
            let ix = v.get("statement_index")?.integer()?;
            if !(0..=500).contains(&ix) {
                return refuse("invalid overlapping statement index");
            }
            let left = v.get("left_operation")?.str()?.to_string();
            let right = v.get("right_operation")?.str()?.to_string();
            let (ld, _, _) = operation_descriptor(&left)?;
            let (rd, _, _) = operation_descriptor(&right)?;
            if ld == rd {
                return refuse(
                    "conflict obligation: both operations write the same semantic effect dimension",
                );
            }
            let index = ix as usize;
            let synthetic = |s: &String| Edit {
                index,
                old: format!("precondition:{s}"),
                new: format!("effect:{s}"),
            };
            (
                synthetic(&left),
                synthetic(&right),
                Some(OperationPair {
                    statement_index: index,
                    left,
                    right,
                }),
                None,
            )
        } else if mode == Some("inferred_ast") {
            let ix = v.get("statement_index")?.integer()?;
            if !(0..=500).contains(&ix) {
                return refuse("invalid inferred AST statement index");
            }
            let index = ix as usize;
            (
                Edit {
                    index,
                    old: "inferred:left".into(),
                    new: "inferred:left:effect".into(),
                },
                Edit {
                    index,
                    old: "inferred:right".into(),
                    new: "inferred:right:effect".into(),
                },
                None,
                Some(index),
            )
        } else if mode.is_none() {
            let left_edit = edit(v.get("left_edit")?)?;
            let right_edit = edit(v.get("right_edit")?)?;
            if left_edit.index == right_edit.index {
                return refuse("semantic overlap: both edits claim the same AST statement slot");
            }
            (left_edit, right_edit, None, None)
        } else {
            return refuse("unknown overlap transaction mode");
        };
        Ok(Self {
            operations,
            inferred,
            base: read_path("base")?,
            left: read_path("left")?,
            right: read_path("right")?,
            output,
            file,
            qualname,
            left_edit,
            right_edit,
            left_checks: suite(&v, "left_checks")?,
            right_checks: suite(&v, "right_checks")?,
            joint_checks: suite(&v, "joint_checks")?,
        })
    }
}

fn rewrite(source: &str, qualname: &str, e: &Edit) -> Result<String> {
    let input = obj(&[
        ("source", Value::string(source)),
        ("qualname", Value::string(qualname)),
        ("index", Value::number(e.index)),
        ("old", Value::string(&e.old)),
        ("new", Value::string(&e.new)),
    ]);
    let mut child = crate::adapter::python_command()
        .arg("-c")
        .arg(include_str!("../../adapters/python_ast_slots.py"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("CPython AST adapter unavailable: {e}"))?;
    child
        .stdin
        .take()
        .ok_or("adapter missing stdin")?
        .write_all(input.canonical().as_bytes())
        .map_err(|e| format!("write adapter request: {e}"))?;
    let result = child
        .wait_with_output()
        .map_err(|e| format!("adapter failed: {e}"))?;
    if !result.status.success() {
        return refuse(format!(
            "AST precondition or rewrite refused: {}",
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    Ok(json::parse(&result.stdout)?
        .get("content")?
        .str()?
        .to_owned())
}

fn rewrite_operation(
    source: &str,
    qualname: &str,
    index: usize,
    operation: &str,
) -> Result<String> {
    operation_descriptor(operation)?;
    let input = obj(&[
        ("source", Value::string(source)),
        ("qualname", Value::string(qualname)),
        ("index", Value::number(index)),
        ("operation", Value::string(operation)),
    ]);
    let mut child = crate::adapter::python_command()
        .arg("-c")
        .arg(include_str!("../../adapters/python_ast_operations.py"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("CPython operation adapter unavailable: {e}"))?;
    child
        .stdin
        .take()
        .ok_or("adapter missing stdin")?
        .write_all(input.canonical().as_bytes())
        .map_err(|e| e.to_string())?;
    let response = child.wait_with_output().map_err(|e| e.to_string())?;
    if !response.status.success() {
        return refuse(format!(
            "typed operation precondition refused: {}",
            String::from_utf8_lossy(&response.stderr)
        ));
    }
    Ok(json::parse(&response.stdout)?
        .get("content")?
        .str()?
        .to_owned())
}

fn infer_source(
    base: &str,
    left: &str,
    right: &str,
    qualname: &str,
    index: usize,
    language: &str,
) -> Result<Value> {
    let request = obj(&[
        ("base", Value::string(base)),
        ("left", Value::string(left)),
        ("right", Value::string(right)),
        ("qualname", Value::string(qualname)),
        ("index", Value::number(index)),
    ]);
    let mut child = if language == "go" {
        let mut c = Command::new("timeout");
        c.arg("25s")
            .arg("go")
            .arg("run")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/adapters/go_infer_effect.go"
            ))
            .env("GOTOOLCHAIN", "local")
            .env("GOPROXY", "off");
        c
    } else if language == "javascript" {
        let mut c = crate::adapter::node_command();
        c.arg("-e")
            .arg(include_str!("../../adapters/js_infer_effect.cjs"));
        c
    } else {
        let mut c = crate::adapter::python_command();
        c.arg("-c")
            .arg(include_str!("../../adapters/python_infer_effect.py"));
        c
    }
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .map_err(|e| format!("native AST inference adapter unavailable: {e}"))?;
    child
        .stdin
        .take()
        .ok_or("adapter stdin missing")?
        .write_all(request.canonical().as_bytes())
        .map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return refuse(format!(
            "inferred effect precondition refused: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let value = json::parse(&out.stdout)?;
    if value.get("calculus")?.str()? != "UNIQUE_SUBEXPRESSION_CONTEXT_V1"
        || !value.get("source_confluence")?.bool()?
        || value.get("statement_index")?.integer()? != index as i64
    {
        return refuse("native effect certificate has unexpected calculus or target");
    }
    for field in [
        "base_statement_sha256",
        "left_statement_sha256",
        "right_statement_sha256",
        "merged_statement_sha256",
        "anchor_before_sha256",
        "anchor_after_sha256",
    ] {
        if !value
            .get(field)?
            .str()?
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
            || value.get(field)?.str()?.len() != 64
        {
            return refuse("malformed inferred effect digest");
        }
    }
    Ok(value)
}

fn apply_action(s: &Spec, source: &str, left: bool) -> Result<String> {
    if let Some(op) = &s.operations {
        rewrite_operation(
            source,
            &s.qualname,
            op.statement_index,
            if left { &op.left } else { &op.right },
        )
    } else {
        rewrite(
            source,
            &s.qualname,
            if left { &s.left_edit } else { &s.right_edit },
        )
    }
}

fn source(root: &Path, commit: &str, path: &str) -> Result<(String, String)> {
    let blob = git::blob(root, commit, path)?;
    let raw = git::command(root, &["cat-file", "blob", &blob])?;
    if digest::git_blob(&raw) != blob {
        return refuse("source bytes disagree with authenticated Git blob OID");
    }
    let text =
        String::from_utf8(raw).map_err(|_| "non-UTF8 Python source unsupported".to_owned())?;
    Ok((blob, text))
}
fn check_mode(root: &Path, commit: &str, file: &str, allow_exec: bool) -> Result<String> {
    let entries = git::entries(root, commit)?;
    let e = entries
        .iter()
        .find(|x| x.path == file)
        .ok_or("missing declared source file")?;
    if !(e.mode == "100644" || (allow_exec && e.mode == "100755")) || e.kind != "blob" {
        return refuse("AST edit requires regular Git blob source mode, never symlinks");
    }
    Ok(e.mode.clone())
}

struct Prepared {
    base: String,
    left: String,
    right: String,
    source_blob: String,
    left_blob: String,
    right_blob: String,
    merged_blob: String,
    merged: String,
    inference: Option<Value>,
}

fn prepare(s: &Spec) -> Result<Prepared> {
    for repo in [&s.base, &s.left, &s.right] {
        git::clean(repo)?;
    }
    if s.output.exists()
        && (s.output.starts_with(&s.base)
            || s.output.starts_with(&s.left)
            || s.output.starts_with(&s.right))
    {
        return refuse("cannot write output inside input checkouts");
    }
    let base = git::rev(&s.base, "HEAD")?;
    let left = git::rev(&s.left, "HEAD")?;
    let right = git::rev(&s.right, "HEAD")?;
    if left == right || git::rev(&s.left, "HEAD^")? != base || git::rev(&s.right, "HEAD^")? != base
    {
        return refuse(
            "both independent branches must be distinct direct children of the same base",
        );
    }
    let mut source_mode = None;
    for (root, commit) in [(&s.base, &base), (&s.left, &left), (&s.right, &right)] {
        let got = check_mode(root, commit, &s.file, s.is_inferred())?;
        if source_mode
            .as_ref()
            .is_some_and(|expected| expected != &got)
        {
            return refuse("undeclared executable-bit or source mode change");
        }
        source_mode = Some(got);
    }
    for (root, branch) in [(&s.left, &left), (&s.right, &right)] {
        if git::changed(root, &base, branch)? != vec![s.file.clone()] {
            return refuse(
                "branch changed undeclared files; cannot authenticate transaction scope",
            );
        }
    }
    let (source_blob, base_source) = source(&s.base, &base, &s.file)?;
    let (_, got_left) = source(&s.left, &left, &s.file)?;
    let (_, got_right) = source(&s.right, &right, &s.file)?;
    let (left_source, right_source, left_then_right, inference) = if let Some(index) = s.inferred {
        let evidence = infer_source(
            &base_source,
            &got_left,
            &got_right,
            &s.qualname,
            index,
            if s.file.ends_with(".py") {
                "python"
            } else if s.file.ends_with(".go") {
                "go"
            } else {
                "javascript"
            },
        )?;
        let combined = evidence.get("content")?.str()?.to_owned();
        (got_left, got_right, combined, Some(evidence))
    } else {
        let left_source = apply_action(s, &base_source, true)?;
        let right_source = apply_action(s, &base_source, false)?;
        if left_source != got_left || right_source != got_right {
            return refuse(
                "branch does not match its single declared typed operation / AST edit exactly",
            );
        }
        let ab = apply_action(s, &left_source, false)?;
        let ba = apply_action(s, &right_source, true)?;
        if ab != ba {
            return refuse("AST operations do not commute in source representation");
        }
        (left_source, right_source, ab, None)
    };
    Ok(Prepared {
        base,
        left,
        right,
        source_blob,
        left_blob: digest::git_blob(left_source.as_bytes()),
        right_blob: digest::git_blob(right_source.as_bytes()),
        merged_blob: digest::git_blob(left_then_right.as_bytes()),
        merged: left_then_right,
        inference,
    })
}

fn tx(kind: &str, p: &Prepared, s: &Spec, commit: &str) -> String {
    let mut value = obj(&[
        ("kind", Value::string(kind)),
        ("base", Value::string(&p.base)),
        ("left", Value::string(&p.left)),
        ("right", Value::string(&p.right)),
        (
            "target",
            Value::string(format!("{}::{}", s.file, s.qualname)),
        ),
        ("result", Value::string(commit)),
        ("source_blob", Value::string(&p.source_blob)),
        ("left_edit", Value::number(s.left_edit.index)),
        ("right_edit", Value::number(s.right_edit.index)),
        ("result_blob", Value::string(&p.merged_blob)),
    ]);
    if let Some(inf) = &p.inference {
        let fields = value.entries_mut().expect("object constructed internally");
        fields.insert(
            "inferred_effect_certificate".into(),
            Value::string(inf.sha256()),
        );
        fields.insert(
            "transaction_calculus".into(),
            Value::string("UNIQUE_SUBEXPRESSION_CONTEXT_V1"),
        );
    }
    if let Some(op) = &s.operations {
        let fields = value.entries_mut().expect("object constructed internally");
        fields.insert("left_operation".into(), Value::string(&op.left));
        fields.insert("right_operation".into(), Value::string(&op.right));
        fields.insert(
            "transaction_calculus".into(),
            Value::string("OVERLAPPING_EFFECT_DIMENSIONS_V1"),
        );
    }
    value.sha256()
}
fn ids(p: &Prepared, s: &Spec, final_commit: &str) -> [String; 3] {
    [
        tx("left_edit", p, s, &p.left),
        tx("right_edit", p, s, &p.right),
        tx("composition", p, s, final_commit),
    ]
}
fn git_clone(base: &Path, output: &Path) -> Result<()> {
    let out = Command::new("git")
        .args(["clone", "-q", "--local", "--no-hardlinks"])
        .arg(base)
        .arg(output)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return refuse(format!(
            "git clone failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}
fn fetch(root: &Path, branch: &Path, commit: &str) -> Result<()> {
    let origin = branch
        .to_str()
        .ok_or("non-UTF8 repository path unsupported")?;
    git::command(root, &["fetch", "-q", origin, commit])?;
    if git::rev(root, commit)? != commit {
        return refuse("fetched branch OID mismatch");
    }
    Ok(())
}
fn native_witnesses(p: &Prepared, s: &Spec, integrated: &Path) -> Result<[Vec<String>; 3]> {
    let left = transactions::native_suite(&s.left, &s.left_checks)?;
    let right = transactions::native_suite(&s.right, &s.right_checks)?;
    let joint = transactions::native_suite(integrated, &s.joint_checks)?;
    for repo in [&s.left, &s.right, integrated] {
        git::clean(repo)?;
    }
    if p.left == p.right {
        return refuse("identical branch commits invalid");
    }
    Ok([left, right, joint])
}
fn rows(
    p: &Prepared,
    s: &Spec,
    merged_commit: &str,
    final_tree: &str,
    witnesses: &[Vec<String>; 3],
) -> Result<EvidenceRows> {
    let id = ids(p, s, merged_commit);
    let target = format!("{}::{}", s.file, s.qualname);
    let lt = git::rev(&s.left, &format!("{}^{{tree}}", p.left))?;
    let rt = git::rev(&s.right, &format!("{}^{{tree}}", p.right))?;
    let mut tx_rows = Vec::new();
    for (i, kind, commit, tree, blob, status) in [
        (
            0,
            "left_edit",
            &p.left,
            &lt,
            &p.left_blob,
            "EXACT_AST_EDIT_AND_NATIVE_CHECK",
        ),
        (
            1,
            "right_edit",
            &p.right,
            &rt,
            &p.right_blob,
            "EXACT_AST_EDIT_AND_NATIVE_CHECK",
        ),
        (
            2,
            "composition",
            &merged_commit.to_owned(),
            &final_tree.to_owned(),
            &p.merged_blob,
            "FRESH_JOINT_VERIFICATION",
        ),
    ] {
        tx_rows.push(vec![
            Some(id[i].clone()),
            Some(kind.to_owned()),
            Some(target.clone()),
            Some(p.base.clone()),
            Some(commit.clone()),
            Some(tree.clone()),
            Some(blob.clone()),
            Some(status.to_owned()),
        ]);
    }
    tx_rows.sort();
    let mut dep_rows = vec![
        vec![
            Some(id[0].clone()),
            Some(id[2].clone()),
            Some("requires".into()),
        ],
        vec![
            Some(id[1].clone()),
            Some(id[2].clone()),
            Some("requires".into()),
        ],
    ];
    dep_rows.sort();
    let mut witness_rows = Vec::new();
    for (i, phase, commit) in [
        (0, "left", p.left.as_str()),
        (1, "right", p.right.as_str()),
        (2, "joint", merged_commit),
    ] {
        for (j, digest) in witnesses[i].iter().enumerate() {
            witness_rows.push(vec![
                Some(id[i].clone()),
                Some(format!("{phase}:{j}")),
                Some(digest.clone()),
                Some(commit.to_owned()),
                Some("0".to_owned()),
            ]);
        }
    }
    witness_rows.sort();
    let mut effect_rows = Vec::new();
    let mut obligation_rows = Vec::new();
    let mut inferred_rows = Vec::new();
    if let Some(op) = &s.operations {
        for (i, kind, output_blob) in [(0, &op.left, &p.left_blob), (1, &op.right, &p.right_blob)] {
            let (_, pre, effect) = operation_descriptor(kind)?;
            effect_rows.push(vec![
                Some(id[i].clone()),
                Some(kind.clone()),
                Some(op.statement_index.to_string()),
                Some(pre.to_owned()),
                Some(effect.to_owned()),
                Some(p.source_blob.clone()),
                Some(output_blob.clone()),
            ]);
        }
        effect_rows.sort();
        let joint = Value::array(witnesses[2].iter().map(Value::string)).sha256();
        for (kind, evidence) in [
            (
                "branch_provenance",
                obj(&[
                    ("left", Value::string(&p.left_blob)),
                    ("right", Value::string(&p.right_blob)),
                ])
                .sha256(),
            ),
            ("source_confluence", p.merged_blob.clone()),
            ("fresh_joint_checks", joint),
        ] {
            obligation_rows.push(vec![
                Some(id[2].clone()),
                Some(kind.to_owned()),
                Some(evidence),
                Some("PASSED".into()),
            ]);
        }
        obligation_rows.sort();
    }
    if let Some(inf) = &p.inference {
        for (idx, role, statement, blob) in [
            (0, "context_rewrite", "left_statement_sha256", &p.left_blob),
            (
                1,
                "subtree_rewrite",
                "right_statement_sha256",
                &p.right_blob,
            ),
        ] {
            inferred_rows.push(vec![
                Some(id[idx].clone()),
                Some(role.into()),
                Some(
                    s.inferred
                        .ok_or("missing inferred statement index")?
                        .to_string(),
                ),
                Some(inf.get("base_statement_sha256")?.str()?.into()),
                Some(inf.get(statement)?.str()?.into()),
                Some(inf.get("anchor_before_sha256")?.str()?.into()),
                Some(inf.get("anchor_after_sha256")?.str()?.into()),
                Some(inf.get("anchor_path")?.canonical()),
                Some(p.source_blob.clone()),
                Some(blob.clone()),
            ]);
        }
        inferred_rows.sort();
        let joint = Value::array(witnesses[2].iter().map(Value::string)).sha256();
        for (kind, evidence) in [
            (
                "branch_provenance",
                obj(&[
                    ("left", Value::string(&p.left_blob)),
                    ("right", Value::string(&p.right_blob)),
                ])
                .sha256(),
            ),
            ("source_confluence", inf.sha256()),
            ("fresh_joint_checks", joint),
        ] {
            obligation_rows.push(vec![
                Some(id[2].clone()),
                Some(kind.into()),
                Some(evidence),
                Some("PASSED".into()),
            ]);
        }
        obligation_rows.sort();
    }
    Ok((
        tx_rows,
        dep_rows,
        witness_rows,
        effect_rows,
        obligation_rows,
        inferred_rows,
    ))
}
fn record(db: &Db, expected: &EvidenceRows) -> Result<()> {
    db.exec(SCHEMA)?;
    if !expected.3.is_empty() || !expected.4.is_empty() {
        db.exec(OP_SCHEMA)?;
    }
    if !expected.5.is_empty() {
        db.exec(INFER_SCHEMA)?;
    }
    db.transaction(|db| {
        for r in &expected.0 {
            let a: Vec<&str> = r.iter().map(|x| x.as_deref().unwrap_or("")).collect();
            db.execute(
                "INSERT INTO overlap_transaction VALUES(?,?,?,?,?,?,?,?)",
                &a.iter().map(|x| Param::Text(x)).collect::<Vec<_>>(),
            )?;
        }
        for r in &expected.1 {
            let a: Vec<&str> = r.iter().map(|x| x.as_deref().unwrap_or("")).collect();
            db.execute(
                "INSERT INTO overlap_dependency VALUES(?,?,?)",
                &a.iter().map(|x| Param::Text(x)).collect::<Vec<_>>(),
            )?;
        }
        for r in &expected.2 {
            let a: Vec<&str> = r.iter().map(|x| x.as_deref().unwrap_or("")).collect();
            db.execute(
                "INSERT INTO overlap_witness VALUES(?,?,?,?,?)",
                &[
                    Param::Text(a[0]),
                    Param::Text(a[1]),
                    Param::Text(a[2]),
                    Param::Text(a[3]),
                    Param::Integer(0),
                ],
            )?;
        }
        for r in &expected.3 {
            let a: Vec<&str> = r.iter().map(|x| x.as_deref().unwrap_or("")).collect();
            db.execute(
                "INSERT INTO operation_effect VALUES(?,?,?,?,?,?,?)",
                &[
                    Param::Text(a[0]),
                    Param::Text(a[1]),
                    Param::Integer(
                        a[2].parse()
                            .map_err(|_| "invalid operation statement index")?,
                    ),
                    Param::Text(a[3]),
                    Param::Text(a[4]),
                    Param::Text(a[5]),
                    Param::Text(a[6]),
                ],
            )?;
        }
        for r in &expected.5 {
            let a: Vec<&str> = r.iter().map(|x| x.as_deref().unwrap_or("")).collect();
            db.execute(
                "INSERT INTO inferred_effect VALUES(?,?,?,?,?,?,?,?,?,?)",
                &a.iter().map(|x| Param::Text(x)).collect::<Vec<_>>(),
            )?;
        }
        for r in &expected.4 {
            let a: Vec<&str> = r.iter().map(|x| x.as_deref().unwrap_or("")).collect();
            db.execute(
                "INSERT INTO composition_obligation VALUES(?,?,?,?)",
                &[
                    Param::Text(a[0]),
                    Param::Text(a[1]),
                    Param::Text(a[2]),
                    Param::Text(a[3]),
                ],
            )?;
        }
        db.foreign_keys_valid()
    })
}
fn compare_db(db: &Db, expected: &EvidenceRows) -> Result<()> {
    db.foreign_keys_valid()?;
    for (query, values, label) in [
        (
            "SELECT * FROM overlap_transaction ORDER BY tx_id",
            &expected.0,
            "transaction",
        ),
        (
            "SELECT * FROM overlap_dependency ORDER BY prerequisite,dependent",
            &expected.1,
            "dependency",
        ),
        (
            "SELECT * FROM overlap_witness ORDER BY tx_id,phase,command_hash",
            &expected.2,
            "witness",
        ),
    ] {
        if db.query(query, &[])? != *values {
            return refuse(format!(
                "independent audit: forged or missing {label} evidence"
            ));
        }
    }
    for (query, expected_rows, label) in [
        (
            "SELECT * FROM operation_effect ORDER BY tx_id",
            &expected.3,
            "typed operation",
        ),
        (
            "SELECT * FROM composition_obligation ORDER BY tx_id,kind",
            &expected.4,
            "composition obligation",
        ),
        (
            "SELECT * FROM inferred_effect ORDER BY tx_id",
            &expected.5,
            "inferred AST effect",
        ),
    ] {
        // A missing table is not evidence of an empty relation. For legacy
        // slot transactions, don't demand tables that were never created.
        if (!expected_rows.is_empty() || (label == "typed operation" && !expected.5.is_empty()))
            && db.query(query, &[])? != *expected_rows
        {
            return refuse(format!(
                "independent audit: forged or missing {label} evidence"
            ));
        }
    }
    Ok(())
}
fn check_final(out: &Path, p: &Prepared, s: &Spec) -> Result<(String, String)> {
    git::clean(out)?;
    let final_commit = git::rev(out, "HEAD")?;
    let final_tree = git::rev(out, &format!("{final_commit}^{{tree}}"))?;
    if git::text(out, &["rev-list", "--parents", "-n", "1", "HEAD"])?
        != format!("{final_commit} {} {}", p.left, p.right)
    {
        return refuse("explanatory merge parents do not match independent branches");
    }
    // Replay the canonical Git commit-tree construction independently. Parent
    // identities and a matching tree alone do not authenticate the explanation.
    let canonical_commit = transactions::git_with_env(
        out,
        &[
            "commit-tree",
            &final_tree,
            "-p",
            &p.left,
            "-p",
            &p.right,
            "-m",
            s.composition_message(),
        ],
    )?;
    if canonical_commit != final_commit {
        return refuse(
            "explanatory commit metadata or message does not match canonical transaction",
        );
    }
    if git::blob(out, &final_commit, &s.file)? != p.merged_blob {
        return refuse("combined Git blob not equal to independently commuting AST rewrites");
    }
    let all = git::changed(out, &p.base, &final_commit)?;
    if all != vec![s.file.clone()] {
        return refuse("merged output changed unexpected tracked paths");
    }
    if check_mode(out, &final_commit, &s.file, s.is_inferred())?
        != check_mode(&s.base, &p.base, &s.file, s.is_inferred())?
    {
        return refuse("integrated Git file mode changed outside declared effect");
    }
    let (_, text) = source(out, &final_commit, &s.file)?;
    if text != p.merged {
        return refuse("merged checkout bytes diverged from validated AST source");
    }
    Ok((final_commit, final_tree))
}
fn report(p: &Prepared, s: &Spec, commit: &str, tree: &str, ids: &[String; 3]) -> Value {
    let mut value = obj(&[
        ("status", Value::string("ACCEPTED_WITH_FRESH_JOINT_CHECKS")),
        (
            "logical_target",
            Value::string(format!("{}::{}", s.file, s.qualname)),
        ),
        ("base_commit", Value::string(&p.base)),
        ("left_commit", Value::string(&p.left)),
        ("right_commit", Value::string(&p.right)),
        ("merge_commit", Value::string(commit)),
        ("final_tree", Value::string(tree)),
        ("final_source_blob", Value::string(&p.merged_blob)),
        ("syntax_commutes", Value::Bool(true)),
        (
            "behavioral_assurance",
            Value::string("FRESH_FINITE_JOINT_CHECKS_ONLY"),
        ),
        ("prior_branch_evidence_carry", Value::string("NOT_CARRIED")),
        (
            "transaction_ids",
            obj(&[
                ("left", Value::string(&ids[0])),
                ("right", Value::string(&ids[1])),
                ("integrated", Value::string(&ids[2])),
            ]),
        ),
        (
            "dependency_ideals",
            Value::array(
                [
                    vec![],
                    vec!["left"],
                    vec!["right"],
                    vec!["left", "right"],
                    vec!["left", "right", "integrated"],
                ]
                .iter()
                .map(|v| Value::array(v.iter().map(|x| Value::string(*x)))),
            ),
        ),
    ]);
    if let Some(inf) = &p.inference {
        let fields = value.entries_mut().expect("object constructed internally");
        fields.insert(
            "calculus".into(),
            Value::string("UNIQUE_SUBEXPRESSION_CONTEXT_V1"),
        );
        fields.insert(
            "derived_effects".into(),
            obj(&[
                ("left", Value::string("context_rewrite")),
                ("right", Value::string("subtree_rewrite")),
                (
                    "anchor_path",
                    inf.get("anchor_path").expect("validated inference").clone(),
                ),
            ]),
        );
        fields.insert(
            "inferred_effect_certificate".into(),
            Value::string(inf.sha256()),
        );
        fields.insert("source_confluence".into(), Value::Bool(true));
        fields.insert(
            "contract_obligations".into(),
            Value::array(
                [
                    "branch_provenance",
                    "source_confluence",
                    "fresh_joint_checks",
                ]
                .into_iter()
                .map(Value::string),
            ),
        );
    }
    if let Some(op) = &s.operations {
        let fields = value.entries_mut().expect("object constructed internally");
        fields.insert(
            "calculus".into(),
            Value::string("OVERLAPPING_EFFECT_DIMENSIONS_V1"),
        );
        fields.insert("left_operation".into(), Value::string(&op.left));
        fields.insert("right_operation".into(), Value::string(&op.right));
        fields.insert("source_confluence".into(), Value::Bool(true));
        fields.insert(
            "contract_obligations".into(),
            Value::array(
                [
                    "branch_provenance",
                    "source_confluence",
                    "fresh_joint_checks",
                ]
                .into_iter()
                .map(Value::string),
            ),
        );
    }
    value
}
pub fn compose(db: &Db, s: &Spec) -> Result<Value> {
    if s.output.exists() {
        return refuse("output already exists");
    }
    if [s.base.as_path(), s.left.as_path(), s.right.as_path()]
        .iter()
        .any(|x| s.output.starts_with(x))
    {
        return refuse("output cannot be nested in source checkout");
    }
    let p = prepare(s)?;
    // Branch obligations freshly executed only after their Git source and AST scope are authenticated.
    let left_w = transactions::native_suite(&s.left, &s.left_checks)?;
    let right_w = transactions::native_suite(&s.right, &s.right_checks)?;
    git::clean(&s.left)?;
    git::clean(&s.right)?;
    git_clone(&s.base, &s.output)?;
    fetch(&s.output, &s.left, &p.left)?;
    fetch(&s.output, &s.right, &p.right)?;
    fs::write(s.output.join(&s.file), &p.merged).map_err(|e| e.to_string())?;
    git::command(&s.output, &["add", "--", &s.file])?;
    let tree = git::rev(&s.output, "HEAD")?; // sanity check base still HEAD
    if tree != p.base {
        return refuse("clone HEAD moved during construction");
    }
    let staged_tree = git::text(&s.output, &["write-tree"])?;
    let commit = transactions::git_with_env(
        &s.output,
        &[
            "commit-tree",
            &staged_tree,
            "-p",
            &p.left,
            "-p",
            &p.right,
            "-m",
            s.composition_message(),
        ],
    )?;
    git::command(&s.output, &["reset", "--hard", &commit])?;
    let (got_commit, got_tree) = check_final(&s.output, &p, s)?;
    if got_commit != commit || got_tree != staged_tree {
        return refuse("commit/tree changed after construction");
    }
    let joint_w = transactions::native_suite(&s.output, &s.joint_checks)?;
    let rechecked = check_final(&s.output, &p, s)?;
    if rechecked != (commit.clone(), staged_tree.clone()) {
        return refuse("native joint tests changed the source checkout or commit identity");
    }
    let witnesses = [left_w, right_w, joint_w];
    let expected = rows(&p, s, &commit, &staged_tree, &witnesses)?;
    record(db, &expected)?;
    let ids = ids(&p, s, &commit);
    let result = report(&p, s, &commit, &staged_tree, &ids);
    fs::write(
        s.output.join(".git").join("relgit_overlap_report.json"),
        result.canonical(),
    )
    .map_err(|e| e.to_string())?;
    Ok(result)
}
pub fn audit(db: &Db, s: &Spec) -> Result<Value> {
    let p = prepare(s)?;
    let (commit, tree) = check_final(&s.output, &p, s)?;
    // Re-run the actual native tests rather than accepting a successful exit recorded in SQLite.
    let witnesses = native_witnesses(&p, s, &s.output)?;
    let expected = rows(&p, s, &commit, &tree, &witnesses)?;
    compare_db(db, &expected)?;
    let ids = ids(&p, s, &commit);
    let result = report(&p, s, &commit, &tree, &ids);
    let report_path = s.output.join(".git").join("relgit_overlap_report.json");
    if fs::read_to_string(&report_path).map_err(|e| e.to_string())? != result.canonical() {
        return refuse("independent audit: report claims differ from Git-derived result");
    }
    Ok(obj(&[
        ("status", Value::string("INDEPENDENT_JOINT_READBACK_PASSED")),
        ("merge_commit", Value::string(&commit)),
        ("final_tree", Value::string(&tree)),
        ("joint_checks", Value::number(s.joint_checks.len())),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_slot_nonoverlap_and_ast_guard() {
        let s = "class X:\n    def f(self):\n        write(b'a')\n        write(b'b')\n";
        let a = Edit {
            index: 0,
            old: "write(b'a')".into(),
            new: "write(bytes((97,)))".into(),
        };
        let b = Edit {
            index: 1,
            old: "write(b'b')".into(),
            new: "write(bytes((98,)))".into(),
        };
        let ab = rewrite(&rewrite(s, "X.f", &a).unwrap(), "X.f", &b).unwrap();
        let ba = rewrite(&rewrite(s, "X.f", &b).unwrap(), "X.f", &a).unwrap();
        assert_eq!(ab, ba);
        assert!(
            rewrite(
                s,
                "X.f",
                &Edit {
                    index: 0,
                    old: "write(b'x')".into(),
                    new: a.new.clone()
                }
            )
            .is_err()
        );
        assert!(
            rewrite(
                s,
                "X.f",
                &Edit {
                    index: 0,
                    old: "write(b'a')".into(),
                    new: "return 0".into()
                }
            )
            .is_err()
        );
    }
    #[test]
    fn typed_operations_converge_on_same_statement() {
        let base =
            "class X:\n    def f(self, data, write):\n        write(b\"%x\\r\\n\" % len(data))\n";
        let a = rewrite_operation(base, "X.f", 0, "codec.ascii_format").unwrap();
        let b = rewrite_operation(base, "X.f", 0, "length.bytes_snapshot").unwrap();
        let ab = rewrite_operation(&a, "X.f", 0, "length.bytes_snapshot").unwrap();
        let ba = rewrite_operation(&b, "X.f", 0, "codec.ascii_format").unwrap();
        assert_eq!(ab, ba);
        assert!(ab.contains("format(len(bytes(data)), \"x\")"));
        assert!(rewrite_operation(&a, "X.f", 0, "codec.ascii_format").is_err());
        assert!(rewrite_operation(base, "X.f", 0, "unknown").is_err());
    }
    #[test]
    fn infer_effects_from_ast_without_operation_names() {
        let base =
            "class X:\n    def f(self, data, write):\n        write(b\"%x\\r\\n\" % len(data))\n";
        let left = base.replace(
            "write(b\"%x\\r\\n\" % len(data))",
            "write(format(len(data), \"x\").encode(\"ascii\") + b\"\\r\\n\")",
        );
        let right = base.replace("len(data))", "len(bytes(data)))");
        let evidence = infer_source(base, &left, &right, "X.f", 0, "python").unwrap();
        assert!(
            evidence
                .get("content")
                .unwrap()
                .str()
                .unwrap()
                .contains("format(len(bytes(data)),")
        );
        let conflict = base.replace("write(b\"%x\\r\\n\" % len(data))", "write(b'constant')");
        assert!(infer_source(base, &right, &conflict, "X.f", 0, "python").is_err());
    }
    #[test]
    fn operation_dimensions_are_explicit() {
        assert_eq!(
            operation_descriptor("codec.ascii_format").unwrap().0,
            "codec"
        );
        assert_eq!(
            operation_descriptor("length.bytes_snapshot").unwrap().0,
            "length"
        );
        assert_eq!(
            operation_descriptor("length.drop_last").unwrap().0,
            "length"
        );
        assert!(operation_descriptor("length.unknown").is_err());
    }
    #[test]
    fn unsupported_input_refusals() {
        assert!(git::validate_path("../escape.py").is_err());
        assert!(json::parse(br#"{"a":1,"a":2}"#).is_err());
    }
}
