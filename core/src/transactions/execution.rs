//! Trusted-local execution and persisted witnesses for composed histories.
//! The repository's Git objects remain authoritative for source identity.
use super::*;
use crate::{historical, index, transform};

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
