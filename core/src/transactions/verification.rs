//! Independent readback of transaction ancestry, stored rows and fresh checks.
//! A passing readback is conditional evidence, not a behavioral theorem.
use super::*;
use crate::index;

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
