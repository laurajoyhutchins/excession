//! Set operations and namespaced views over authenticated source identities.
//! Every operation produces a deterministic manifest; none copies source files.
use super::*;

fn same_revision(left: &Plan, right: &Plan) -> Result<()> {
    if left.commit != right.commit || left.tree != right.tree {
        return refuse("cannot compose projections from different Git source revisions");
    }
    Ok(())
}

fn source_identity(e: &Entry) -> (&str, &str, &str) {
    (&e.source_path, &e.blob, &e.mode)
}

/// Compose two independently checked view manifests by *source identity*, never
/// by substring matching their display names. Multiple virtual references to
/// one source blob are meaningful and retained when selected.
pub fn compose(repo: &Path, operator: &str, left: &Path, right: &Path) -> Result<Value> {
    if !matches!(operator, "union" | "intersection" | "difference") {
        return refuse("view composition supports union, intersection, difference only");
    }
    let a = checked_plan(repo, left)?;
    let b = checked_plan(repo, right)?;
    same_revision(&a, &b)?;
    let a_id = a.payload().sha256();
    let b_id = b.payload().sha256();
    let left_sources = a
        .entries
        .iter()
        .map(source_identity)
        .collect::<BTreeSet<_>>();
    let right_sources = b
        .entries
        .iter()
        .map(source_identity)
        .collect::<BTreeSet<_>>();
    let mut selected = BTreeMap::<String, Entry>::new();
    for e in &a.entries {
        let keep = match operator {
            "union" => true,
            "intersection" => right_sources.contains(&source_identity(e)),
            "difference" => !right_sources.contains(&source_identity(e)),
            _ => unreachable!(),
        };
        if keep {
            selected.insert(e.virtual_path.clone(), e.clone());
        }
    }
    if operator == "union" {
        for e in &b.entries {
            // Same virtual pathname with different provenance/meaning cannot be
            // silently overwritten, even when the two blobs are byte-identical.
            if let Some(previous) = selected.get(&e.virtual_path) {
                if previous != e {
                    return refuse(format!("conflicting virtual view path: {}", e.virtual_path));
                }
            } else {
                selected.insert(e.virtual_path.clone(), e.clone());
            }
        }
    }
    // Sanity check ensures we have a source-based operation, rather than a
    // fortuitous equal count of unrelated directory entries.
    if operator == "intersection" && left_sources.is_disjoint(&right_sources) {
        return refuse("view intersection has no common source identity");
    }
    let query = obj(&[
        ("operator", Value::string(operator)),
        ("left_view_id", Value::string(a_id)),
        ("right_view_id", Value::string(b_id)),
        ("selection", Value::string("source-identity-v1")),
    ]);
    let plan = make_plan(
        "composed",
        &format!("query:{}", query.sha256()),
        a.commit,
        a.tree,
        selected.into_values().collect(),
    )?;
    Ok(plan.manifest())
}

/// Narrow an existing projection by a precise role or path-subtree. The
/// filter is deterministic and does not rely on a language parser.
pub fn select(repo: &Path, manifest: &Path, axis: &str, needle: &str) -> Result<Value> {
    if needle.is_empty() || needle.len() > 512 {
        return refuse("view filter value must contain 1..512 characters");
    }
    if !matches!(axis, "role" | "source-prefix" | "virtual-prefix") {
        return refuse("supported view filter axes: role, source-prefix, virtual-prefix");
    }
    if axis != "role" {
        git::validate_path(needle)?;
    }
    let input = checked_plan(repo, manifest)?;
    let input_id = input.payload().sha256();
    let entries = input
        .entries
        .into_iter()
        .filter(|e| match axis {
            "role" => e.role == needle,
            "source-prefix" => {
                e.source_path == needle || e.source_path.starts_with(&format!("{needle}/"))
            }
            "virtual-prefix" => {
                e.virtual_path == needle || e.virtual_path.starts_with(&format!("{needle}/"))
            }
            _ => unreachable!(),
        })
        .collect::<Vec<_>>();
    let query = obj(&[
        ("operator", Value::string("select")),
        ("input_view_id", Value::string(input_id)),
        ("axis", Value::string(axis)),
        ("value", Value::string(needle)),
    ]);
    Ok(make_plan(
        "composed",
        &format!("query:{}", query.sha256()),
        input.commit,
        input.tree,
        entries,
    )?
    .manifest())
}

/// Place a view below a separate named virtual subtree. This permits two
/// independently focused layouts to coexist even if their original paths clash.
pub fn mount(repo: &Path, manifest: &Path, namespace: &str) -> Result<Value> {
    if namespace.is_empty()
        || namespace.len() > 64
        || !namespace
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return refuse("view namespace must be 1..64 ASCII letters, digits, '-' or '_'");
    }
    let input = checked_plan(repo, manifest)?;
    let query = obj(&[
        ("operator", Value::string("mount")),
        ("input_view_id", Value::string(input.payload().sha256())),
        ("namespace", Value::string(namespace)),
    ]);
    let mut entries = input.entries;
    for e in &mut entries {
        let tail = e
            .virtual_path
            .strip_prefix("views/")
            .ok_or("virtual entry missing views/ root")?;
        e.virtual_path = format!("views/{namespace}/{tail}");
    }
    Ok(make_plan(
        "composed",
        &format!("query:{}", query.sha256()),
        input.commit,
        input.tree,
        entries,
    )?
    .manifest())
}

pub fn mount_replay(repo: &Path, input: &Path, namespace: &str, output: &Path) -> Result<Value> {
    replay(&mount(repo, input, namespace)?, output)
}

pub fn compose_replay(
    repo: &Path,
    operator: &str,
    left: &Path,
    right: &Path,
    output: &Path,
) -> Result<Value> {
    replay(&compose(repo, operator, left, right)?, output)
}

pub fn select_replay(
    repo: &Path,
    input: &Path,
    axis: &str,
    value: &str,
    output: &Path,
) -> Result<Value> {
    replay(&select(repo, input, axis, value)?, output)
}

