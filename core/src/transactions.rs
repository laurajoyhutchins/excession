//! Semantic transactions: compose a verified structural projection and a separately
//! developed old-layout defect fix, with a real two-parent explanatory Git commit.
//! Local, trusted execution only. Evidence is finite and conditional, not a theorem.
//! Test receipts record command identity and exit status, not nondeterministic pytest stdout.
use crate::sqlite::{Db, Param};
use crate::{Result, git, historical, index, json, refuse, transform};
use json::{Value, obj};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS semantic_transaction (
  tx_id TEXT PRIMARY KEY,
  kind TEXT NOT NULL CHECK(kind IN ('module_move','historical_fix','composition')),
  logical_target TEXT NOT NULL,
  source_commit TEXT NOT NULL,
  result_commit TEXT NOT NULL,
  result_tree TEXT NOT NULL,
  evidence_status TEXT NOT NULL CHECK(evidence_status IN ('BOUNDED_RUNTIME_CHECK','EXACT_HISTORICAL_BLOBS','JOINT_CHECKS_PASSED'))
);
CREATE TABLE IF NOT EXISTS semantic_dependency (
  prerequisite TEXT NOT NULL REFERENCES semantic_transaction(tx_id),
  dependent TEXT NOT NULL REFERENCES semantic_transaction(tx_id),
  kind TEXT NOT NULL CHECK(kind IN ('requires')),
  PRIMARY KEY(prerequisite, dependent),
  CHECK(prerequisite != dependent)
);
CREATE TABLE IF NOT EXISTS semantic_witness (
  tx_id TEXT NOT NULL REFERENCES semantic_transaction(tx_id),
  phase TEXT NOT NULL,
  command_digest TEXT NOT NULL,
  input_commit TEXT NOT NULL,
  observed_exit INTEGER NOT NULL CHECK(observed_exit=0),
  PRIMARY KEY(tx_id, phase, command_digest)
);
"#;

#[derive(Clone)]
pub struct Spec {
    pub base: PathBuf,
    pub fix: PathBuf,
    pub output: PathBuf,
    pub name: String,
    pub logical: String,
    pub module_old: String,
    pub module_new: String,
    pub adapter: PathBuf,
    pub manifest: PathBuf,
    pub layout_probe: Vec<String>,
    pub layout_compat: Vec<String>,
    pub fix_checks: Vec<Vec<String>>,
    pub transported_checks: Vec<Vec<String>>,
    pub joint_checks: Vec<Vec<String>>,
}
fn get_cmd(v: &Value) -> Result<Vec<String>> {
    let row = v.array_ref()?;
    if row.is_empty() || row.len() > 16 {
        return refuse("empty or oversized native test command");
    }
    let argv: Vec<String> = row
        .iter()
        .map(|x| Ok(x.str()?.to_owned()))
        .collect::<Result<_>>()?;
    if !matches!(argv[0].as_str(), "python" | "python3" | "node" | "go") {
        return refuse("only reviewed native Python/Node commands supported");
    }
    if argv.iter().any(|x| x.is_empty() || x.contains('\0')) {
        return refuse("invalid native argv");
    }
    if argv[0] == "go" && !matches!(argv.get(1).map(String::as_str), Some("test" | "run")) {
        return refuse("Go native checks restricted to go test/run");
    }
    Ok(argv)
}
fn get_suite(v: &Value, name: &str) -> Result<Vec<Vec<String>>> {
    let vals = v.get(name)?.array_ref()?;
    if vals.is_empty() || vals.len() > 16 {
        return refuse(format!(
            "{name}: at least one and at most 16 checks required"
        ));
    }
    vals.iter().map(get_cmd).collect()
}
impl Spec {
    pub fn read(path: &Path) -> Result<Self> {
        let v = json::parse(&fs::read(path).map_err(|e| e.to_string())?)?;
        if !v.get("trusted_local")?.bool()? {
            return refuse("trusted_local must be explicitly true");
        }
        let path_field = |key: &str| -> Result<PathBuf> {
            PathBuf::from(v.get(key)?.str()?)
                .canonicalize()
                .map_err(|e| format!("{key}: {e}"))
        };
        let output = PathBuf::from(v.get("output")?.str()?);
        if !output.is_absolute() {
            return refuse("output must be an absolute directory");
        }
        let name = v.get("name")?.str()?.to_string();
        if name.trim().is_empty() || name.contains('/') {
            return refuse("invalid package root");
        }
        let logical = v.get("logical")?.str()?.to_string();
        let module_old = v.get("module_old")?.str()?.to_string();
        let module_new = v.get("module_new")?.str()?.to_string();
        git::validate_path(&module_old)?;
        git::validate_path(&module_new)?;
        if module_old == module_new {
            return refuse("no structural change");
        }
        if !module_old.starts_with(&format!("{name}/"))
            || !module_new.starts_with(&format!("{name}/"))
        {
            return refuse("moved module must remain in declared package");
        }
        let layout_probe = get_cmd(v.get("layout_probe")?)?;
        let layout_compat = get_cmd(v.get("layout_compat")?)?;
        Ok(Self {
            base: path_field("base")?,
            fix: path_field("fix")?,
            adapter: path_field("adapter")?,
            manifest: path_field("manifest")?,
            output,
            name,
            logical,
            module_old,
            module_new,
            layout_probe,
            layout_compat,
            fix_checks: get_suite(&v, "fix_checks")?,
            transported_checks: get_suite(&v, "transported_checks")?,
            joint_checks: get_suite(&v, "joint_checks")?,
        })
    }
}
fn native_check(root: &Path, argv: &[String]) -> Result<String> {
    if !matches!(
        argv.first().map(String::as_str),
        Some("python" | "python3" | "node" | "go")
    ) {
        return refuse("unsupported native command");
    }
    let out = Command::new("timeout")
        .arg("--kill-after=2s")
        .arg("12s")
        .arg(&argv[0])
        .args(&argv[1..])
        .current_dir(root)
        .env("GOTOOLCHAIN", "local")
        .env("GOPROXY", "off")
        .env("PYTHONPATH", root)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("PYTEST_DISABLE_PLUGIN_AUTOLOAD", "1")
        .output()
        .map_err(|e| format!("native check launch failed: {e}"))?;
    if !out.status.success() {
        return refuse(format!(
            "native check failed at {}: {} {}",
            root.display(),
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        ));
    }
    Ok(obj(&[
        ("command", Value::array(argv.iter().map(Value::string))),
        ("exit", Value::number(0)),
    ])
    .sha256())
}
pub(crate) fn native_suite(root: &Path, suite: &[Vec<String>]) -> Result<Vec<String>> {
    suite.iter().map(|argv| native_check(root, argv)).collect()
}
pub(crate) fn git_with_env(root: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_AUTHOR_NAME", "RelGit Semantic Transaction")
        .env("GIT_COMMITTER_NAME", "RelGit Semantic Transaction")
        .env("GIT_AUTHOR_EMAIL", "relgit@example.invalid")
        .env("GIT_COMMITTER_EMAIL", "relgit@example.invalid")
        .env("GIT_AUTHOR_DATE", "2026-11-09T00:00:00+0000")
        .env("GIT_COMMITTER_DATE", "2026-11-09T00:00:00+0000")
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return refuse(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    String::from_utf8(out.stdout)
        .map(|x| x.trim().to_owned())
        .map_err(|e| e.to_string())
}
fn tx(kind: &str, logical: &str, input: &str, output: &str) -> String {
    obj(&[
        ("kind", Value::string(kind)),
        ("logical", Value::string(logical)),
        ("input", Value::string(input)),
        ("output", Value::string(output)),
    ])
    .sha256()
}

/// Order ideals for A, B incomparable, both required by integrated result C.
/// Their semantics reflect prerequisites, not chronological Git order.
pub fn dependency_ideals() -> Vec<Vec<&'static str>> {
    vec![
        vec![],
        vec!["structural"],
        vec!["behavioral"],
        vec!["structural", "behavioral"],
        vec!["structural", "behavioral", "integrated"],
    ]
}

pub fn compose(db: &Db, s: &Spec) -> Result<Value> {
    if s.output.exists() {
        return refuse("output already exists");
    }
    git::clean(&s.base)?;
    git::clean(&s.fix)?;
    let base_commit = git::rev(&s.base, "HEAD")?;
    let fix_commit = git::rev(&s.fix, "HEAD")?;
    let fix_parent = git::rev(&s.fix, "HEAD^")?;
    if fix_parent != base_commit || base_commit == fix_commit {
        return refuse("independent behavioral change must be a direct child of same base");
    }
    if s.output.starts_with(&s.base)
        || s.output.starts_with(&s.fix)
        || s.base.starts_with(&s.output)
        || s.fix.starts_with(&s.output)
    {
        return refuse("transaction output must be separate from inputs");
    }
    // Independent branch assurances are freshly executed, never inferred from a green status file.
    let fix_witnesses = native_suite(&s.fix, &s.fix_checks)?;
    // Refuse obviously unverifiable plans before creating output commits.
    if s.transported_checks.is_empty() || s.joint_checks.is_empty() {
        return refuse("both transported and joint tests required");
    }
    fs::create_dir_all(&s.output).map_err(|e| e.to_string())?;
    let structural_out = s.output.join("structural");
    let replay_out = s.output.join("mapped_replay");
    let final_out = s.output.join("integrated");
    let plan = transform::Plan {
        source: s.base.clone(),
        name: s.name.clone(),
        base: base_commit.clone(),
        language: if s.module_old.ends_with(".py") {
            "python"
        } else {
            "javascript"
        }
        .into(),
        moves: vec![transform::Move {
            logical: s.logical.clone(),
            old: s.module_old.clone(),
            new: s.module_new.clone(),
        }],
        probe: s.layout_probe.clone(),
        compatibility: s.layout_compat.clone(),
        adapter_path: s.adapter.clone(),
    };
    let projected = transform::project(db, &plan, &structural_out)?;
    let projected_commit = projected.get("projection_commit")?.str()?.to_owned();
    let projected_tree = projected.get("projection_tree")?.str()?.to_owned();
    let projection_id = projected.get("id")?.str()?.to_owned();
    let replay = historical::ReplayPlan {
        source_index_name: Some(format!("{}-behavioral", s.name)),
        manifest: historical::from_spec(&{
            // Parse the original manifest through the established, validated importer.
            let replay_spec = s.output.join("replay_input.json");
            fs::write(
                &replay_spec,
                obj(&[
                    ("source", Value::string(s.fix.to_string_lossy())),
                    (
                        "projection",
                        Value::string(structural_out.to_string_lossy()),
                    ),
                    ("projection_id", Value::string(&projection_id)),
                    ("manifest", Value::string(s.manifest.to_string_lossy())),
                    ("name", Value::string(&s.name)),
                    ("output", Value::string(replay_out.to_string_lossy())),
                    (
                        "native_tests",
                        Value::array(
                            s.transported_checks
                                .iter()
                                .map(|c| Value::array(c.iter().map(Value::string))),
                        ),
                    ),
                ])
                .canonical(),
            )
            .map_err(|e| e.to_string())?;
            replay_spec
        })?
        .manifest,
        source: s.fix.clone(),
        projection: structural_out.clone(),
        projection_id: projection_id.clone(),
        output: replay_out.clone(),
        name: s.name.clone(),
        native_tests: s.transported_checks.clone(),
    };
    let replayed = historical::replay(db, &replay)?;
    let replay_commit = replayed.get("projected_commit")?.str()?.to_owned();
    let result_tree = replayed.get("projected_tree")?.str()?.to_owned();
    let transported_witnesses = native_suite(&replay_out, &s.transported_checks)?;
    let joint_witnesses = native_suite(&replay_out, &s.joint_checks)?;
    // Build a provenance-preserving merge: parents are the independently developed
    // branches. Its tree is the separately reviewed, mapped three-way patch result.
    let clone = Command::new("git")
        .args(["clone", "--local", "--no-hardlinks", "--quiet"])
        .arg(&replay_out)
        .arg(&final_out)
        .output()
        .map_err(|e| e.to_string())?;
    if !clone.status.success() {
        return refuse(format!(
            "final clone: {}",
            String::from_utf8_lossy(&clone.stderr)
        ));
    }
    git::clean(&final_out)?;
    if git::rev(&final_out, "HEAD")? != replay_commit {
        return refuse("final checkout not at mapped replay");
    }
    let fetch = Command::new("git")
        .arg("-C")
        .arg(&final_out)
        .arg("fetch")
        .arg("--no-tags")
        .arg("--quiet")
        .arg(&s.fix)
        .arg(&fix_commit)
        .output()
        .map_err(|e| e.to_string())?;
    if !fetch.status.success() {
        return refuse(format!(
            "import second parent failed: {}",
            String::from_utf8_lossy(&fetch.stderr)
        ));
    }
    let merge_base = git::text(&final_out, &["merge-base", &projected_commit, &fix_commit])?;
    if merge_base != base_commit {
        return refuse("independent parents do not share declared merge base");
    }
    let explanation = format!(
        "relgit: compose {}\n\nstructural: {}\nbehavioral: {}\nprojection: {}\nmapped-replay: {}\nverified: finite native checks, no universal equivalence\n",
        s.logical, projected_commit, fix_commit, projection_id, replay_commit
    );
    let merge_commit = git_with_env(
        &final_out,
        &[
            "commit-tree",
            &result_tree,
            "-p",
            &projected_commit,
            "-p",
            &fix_commit,
            "-m",
            &explanation,
        ],
    )?;
    git::oid(&merge_commit)?;
    git::command(
        &final_out,
        &["update-ref", "HEAD", &merge_commit, &replay_commit],
    )?;
    git::clean(&final_out)?;
    if git::rev(&final_out, &format!("{merge_commit}^{{tree}}"))? != result_tree
        || git::text(
            &final_out,
            &["rev-list", "--parents", "-n", "1", &merge_commit],
        )? != format!("{merge_commit} {projected_commit} {fix_commit}")
    {
        return refuse("explanatory merge ancestry/tree incorrect");
    }
    let final_witnesses = native_suite(&final_out, &s.joint_checks)?;
    // The final snapshot has exactly the mapped replay Git tree. No source rewriter ran after checks.
    let repo_id = index::index_revision_with_roots(
        db,
        &final_out,
        &format!("{}-composed", s.name),
        &merge_commit,
        "verified semantic composition",
        &[&s.name, "tests"],
    )?;
    if index::reconstruct_tree(db, repo_id, &merge_commit)? != result_tree {
        return refuse("SQL vs final Git tree mismatch");
    }
    let a = tx("module_move", &s.logical, &base_commit, &projected_commit);
    let b = tx("historical_fix", &s.logical, &base_commit, &fix_commit);
    let c = tx("composition", &s.logical, &base_commit, &merge_commit);
    db.exec(SCHEMA)?;
    db.transaction(|db| {
        for (id, kind, input, output, tree, status) in [
            (
                &a,
                "module_move",
                &base_commit,
                &projected_commit,
                &projected_tree,
                "BOUNDED_RUNTIME_CHECK",
            ),
            (
                &b,
                "historical_fix",
                &base_commit,
                &fix_commit,
                &git::rev(&s.fix, &format!("{fix_commit}^{{tree}}"))?,
                "EXACT_HISTORICAL_BLOBS",
            ),
            (
                &c,
                "composition",
                &base_commit,
                &merge_commit,
                &result_tree,
                "JOINT_CHECKS_PASSED",
            ),
        ] {
            db.execute(
                "INSERT INTO semantic_transaction VALUES(?,?,?,?,?,?,?)",
                &[
                    Param::Text(id),
                    Param::Text(kind),
                    Param::Text(&s.logical),
                    Param::Text(input),
                    Param::Text(output),
                    Param::Text(tree),
                    Param::Text(status),
                ],
            )?;
        }
        for pre in [&a, &b] {
            db.execute(
                "INSERT INTO semantic_dependency VALUES(?,?,'requires')",
                &[Param::Text(pre), Param::Text(&c)],
            )?;
        }
        for (id, phase, input, digests) in [
            (&b, "fix_branch", &fix_commit, &fix_witnesses),
            (&c, "mapped_checks", &replay_commit, &transported_witnesses),
            (&c, "joint_before_commit", &replay_commit, &joint_witnesses),
            (&c, "joint_final", &merge_commit, &final_witnesses),
        ] {
            for (i, digest) in digests.iter().enumerate() {
                let phase_name = format!("{phase}:{i}");
                db.execute(
                    "INSERT INTO semantic_witness VALUES(?,?,?,?,0)",
                    &[
                        Param::Text(id),
                        Param::Text(&phase_name),
                        Param::Text(digest),
                        Param::Text(input),
                    ],
                )?;
            }
        }
        db.foreign_keys_valid()
    })?;
    let report = obj(&[
        (
            "status",
            Value::string("ACCEPTED_BOUNDED_JOINT_VERIFICATION"),
        ),
        ("logical_target", Value::string(&s.logical)),
        ("base_commit", Value::string(&base_commit)),
        ("structural_commit", Value::string(&projected_commit)),
        ("behavioral_commit", Value::string(&fix_commit)),
        ("mapped_replay_commit", Value::string(&replay_commit)),
        ("explanatory_merge_commit", Value::string(&merge_commit)),
        ("final_tree", Value::string(&result_tree)),
        (
            "transaction_ids",
            obj(&[
                ("structural", Value::string(&a)),
                ("behavioral", Value::string(&b)),
                ("integrated", Value::string(&c)),
            ]),
        ),
        (
            "prerequisite_order_ideals",
            Value::array(
                dependency_ideals()
                    .iter()
                    .map(|xs| Value::array(xs.iter().map(|x| Value::string(*x)))),
            ),
        ),
        ("fix_branch_checks", Value::number(fix_witnesses.len())),
        (
            "transported_checks",
            Value::number(transported_witnesses.len()),
        ),
        ("joint_checks", Value::number(joint_witnesses.len())),
        (
            "source_identity_carry",
            Value::string("NOT_CARRIED_ACROSS_MODULE_MOVE"),
        ),
        (
            "evidence_scope",
            Value::string("fresh finite runtime checks; unsigned local evidence"),
        ),
    ]);
    fs::write(s.output.join("semantic_report.json"), report.canonical())
        .map_err(|e| e.to_string())?;
    Ok(report)
}

/// Independently recompute accepted claims from Git objects, SQLite rows and freshly
/// executed native checks. The receipt is unsigned and the executor is trusted local.
pub fn audit(db: &Db, s: &Spec) -> Result<Value> {
    db.exec(SCHEMA)?;
    let structural = s.output.join("structural");
    let mapped = s.output.join("mapped_replay");
    let integrated = s.output.join("integrated");
    for repo in [&s.base, &s.fix, &structural, &mapped, &integrated] {
        git::clean(repo)?;
    }
    let base_commit = git::rev(&s.base, "HEAD")?;
    let fixed = git::rev(&s.fix, "HEAD")?;
    let struct_commit = git::rev(&structural, "HEAD")?;
    let mapped_commit = git::rev(&mapped, "HEAD")?;
    let merged_commit = git::rev(&integrated, "HEAD")?;
    let tree = git::rev(&integrated, &format!("{merged_commit}^{{tree}}"))?;
    let parents = git::text(
        &integrated,
        &["rev-list", "--parents", "-n", "1", &merged_commit],
    )?;
    if parents != format!("{merged_commit} {struct_commit} {fixed}")
        || git::rev(&structural, "HEAD^")? != base_commit
        || git::rev(&s.fix, "HEAD^")? != base_commit
        || git::rev(&mapped, "HEAD^")? != struct_commit
        || git::rev(&mapped, &format!("{mapped_commit}^{{tree}}"))? != tree
    {
        return refuse("Git source ancestry or mapped/final trees disagree");
    }
    let expected = [
        (
            tx("module_move", &s.logical, &base_commit, &struct_commit),
            "module_move",
            base_commit.as_str(),
            struct_commit.as_str(),
            git::rev(&structural, &format!("{struct_commit}^{{tree}}"))?,
            "BOUNDED_RUNTIME_CHECK",
        ),
        (
            tx("historical_fix", &s.logical, &base_commit, &fixed),
            "historical_fix",
            base_commit.as_str(),
            fixed.as_str(),
            git::rev(&s.fix, &format!("{fixed}^{{tree}}"))?,
            "EXACT_HISTORICAL_BLOBS",
        ),
        (
            tx("composition", &s.logical, &base_commit, &merged_commit),
            "composition",
            base_commit.as_str(),
            merged_commit.as_str(),
            tree.clone(),
            "JOINT_CHECKS_PASSED",
        ),
    ];
    if db
        .query("SELECT count(*) FROM semantic_transaction", &[])?
        .first()
        .and_then(|r| r[0].as_deref())
        != Some("3")
        || db
            .query("SELECT count(*) FROM semantic_dependency", &[])?
            .first()
            .and_then(|r| r[0].as_deref())
            != Some("2")
    {
        return refuse("semantic transaction graph has unexpected cardinality");
    }
    for (id, kind, input, output, t, status) in &expected {
        let rows=db.query("SELECT kind,logical_target,source_commit,result_commit,result_tree,evidence_status FROM semantic_transaction WHERE tx_id=?",&[Param::Text(id)])?;
        if rows.len() != 1
            || rows[0]
                != vec![
                    Some((*kind).into()),
                    Some(s.logical.clone()),
                    Some((*input).into()),
                    Some((*output).into()),
                    Some(t.clone()),
                    Some((*status).into()),
                ]
        {
            return refuse(format!("semantic transaction row mismatch: {kind}"));
        }
    }
    let rows = db.query(
        "SELECT prerequisite,dependent,kind FROM semantic_dependency ORDER BY prerequisite",
        &[],
    )?;
    let mut want = vec![
        vec![
            Some(expected[0].0.clone()),
            Some(expected[2].0.clone()),
            Some("requires".into()),
        ],
        vec![
            Some(expected[1].0.clone()),
            Some(expected[2].0.clone()),
            Some("requires".into()),
        ],
    ];
    want.sort();
    if rows != want {
        return refuse("logical prerequisite graph does not match two independent branches");
    }
    let id = db.query(
        "SELECT id FROM repository WHERE name=?",
        &[Param::Text(&format!("{}-composed", s.name))],
    )?;
    if id.len() != 1 {
        return refuse("missing composed repository in SQLite catalog");
    }
    let repo_id = id[0][0]
        .as_deref()
        .ok_or("null repository ID")?
        .parse::<i64>()
        .map_err(|e| e.to_string())?;
    if index::reconstruct_tree(db, repo_id, &merged_commit)? != tree {
        return refuse("indexed repository tree not identical to merged Git tree");
    }
    let moved = db.query(
        "SELECT old_path,new_path FROM projection_move WHERE module_name=?",
        &[Param::Text(&s.logical)],
    )?;
    if moved.len() != 1 || moved[0] != vec![Some(s.module_old.clone()), Some(s.module_new.clone())]
    {
        return refuse("projection mapping not identical to logical module move");
    }
    let replay_paths = db.query(
        "SELECT original_path,mapped_path,exact_upstream FROM replay_path ORDER BY original_path",
        &[],
    )?;
    if replay_paths.is_empty()
        || !replay_paths.iter().any(|r| {
            r[0].as_deref() == Some(&s.module_old)
                && r[1].as_deref() == Some(&s.module_new)
                && r[2].as_deref() == Some("1")
        })
    {
        return refuse("transported historical change lacks exact source witness");
    }
    let observed = [
        (
            &expected[1].0,
            "fix_branch",
            &fixed,
            native_suite(&s.fix, &s.fix_checks)?,
        ),
        (
            &expected[2].0,
            "mapped_checks",
            &mapped_commit,
            native_suite(&mapped, &s.transported_checks)?,
        ),
        (
            &expected[2].0,
            "joint_before_commit",
            &mapped_commit,
            native_suite(&mapped, &s.joint_checks)?,
        ),
        (
            &expected[2].0,
            "joint_final",
            &merged_commit,
            native_suite(&integrated, &s.joint_checks)?,
        ),
    ];
    let receipt_count: usize = observed.iter().map(|x| x.3.len()).sum();
    let count = db.query("SELECT count(*) FROM semantic_witness", &[])?;
    if count
        .first()
        .and_then(|r| r[0].as_deref())
        .ok_or("missing witness count")?
        .parse::<usize>()
        .map_err(|e| e.to_string())?
        != receipt_count
    {
        return refuse("semantic witness count differs from declared checks");
    }
    for (id, phase, input, checks) in observed {
        for (i, h) in checks.iter().enumerate() {
            let phase = format!("{phase}:{i}");
            let rows=db.query("SELECT command_digest,input_commit,observed_exit FROM semantic_witness WHERE tx_id=? AND phase=?",&[Param::Text(id),Param::Text(&phase)])?;
            if rows != vec![vec![Some(h.clone()), Some(input.clone()), Some("0".into())]] {
                return refuse(format!("native witness mismatch for {phase}"));
            }
        }
    }
    db.foreign_keys_valid()?;
    Ok(obj(&[
        ("status", Value::string("INDEPENDENT_READBACK_PASSED")),
        ("merge_commit", Value::string(merged_commit)),
        ("git_tree", Value::string(tree)),
        ("native_witnesses_rechecked", Value::number(receipt_count)),
        ("logical_states", Value::number(dependency_ideals().len())),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn poset_has_five_valid_ideals() {
        let ideals = dependency_ideals();
        assert_eq!(ideals.len(), 5);
        assert!(!ideals.contains(&vec!["integrated"]));
        assert!(ideals.contains(&vec!["structural", "behavioral", "integrated"]));
    }
    #[test]
    fn only_explicit_native_runtimes() {
        assert!(
            get_cmd(&Value::array(vec![
                Value::string("sh"),
                Value::string("-c")
            ]))
            .is_err()
        );
        assert!(
            get_cmd(&Value::array(vec![
                Value::string("python"),
                Value::string("-c")
            ]))
            .is_ok()
        );
        assert!(get_cmd(&Value::array(vec![])).is_err());
    }
}
