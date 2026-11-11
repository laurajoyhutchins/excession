//! Transparent, language-neutral workload cost policy.
//! Measured inputs are supplied by native workload runners; this is not a
//! learned optimizer nor a proof of an optimal layout.
use crate::{
    Result,
    json::{self, Value, obj},
    refuse,
};
use std::{fs, path::Path};

#[derive(Debug, Clone)]
struct Candidate {
    verified: bool,
    edit_paths: u64,
    cross_edges: u64,
    runtime_ms: f64,
    migration_paths: u64,
    conflicts: u64,
}
fn num(v: &Value) -> Result<f64> {
    let s = match v {
        Value::Number(n) => n,
        _ => return refuse("expected numeric JSON field"),
    };
    let n = s.parse::<f64>().map_err(|_| "invalid number")?;
    if !n.is_finite() {
        return refuse("non-finite policy measurement");
    }
    Ok(n)
}
fn integer(v: &Value) -> Result<u64> {
    let n = v.integer()?;
    u64::try_from(n).map_err(|_| "negative count".into())
}
fn candidate(v: &Value) -> Result<Candidate> {
    Ok(Candidate {
        verified: v.get("verified")?.bool()?,
        edit_paths: integer(v.get("source_edit_paths")?)?,
        cross_edges: integer(v.get("cross_package_edges")?)?,
        runtime_ms: num(v.get("median_runtime_ms")?)?,
        migration_paths: integer(v.get("migration_paths")?)?,
        conflicts: integer(v.get("merge_conflicts")?)?,
    })
}
fn field(v: &Value, name: &str, default: f64) -> Result<f64> {
    match v.entries()?.get(name) {
        Some(x) => num(x),
        None => Ok(default),
    }
}
fn round4(x: f64) -> Result<f64> {
    if !x.is_finite() || x.abs() > 9e12 {
        return refuse("cost exceeds stable numeric range");
    }
    Ok((x * 10000.).round() / 10000.)
}
fn fvalue(n: f64) -> Value {
    Value::Number(format!("{n:.4}"))
}
/// Pure cost model: fail closed for malformed policy or unverified baseline.
pub fn choose(v: &Value) -> Result<Value> {
    let a = candidate(v.get("original")?)?;
    let b = candidate(v.get("projected")?)?;
    if !a.verified {
        return refuse("baseline unverified");
    }
    let p = v.get("policy")?;
    let horizon = match p.entries()?.get("future_edits") {
        Some(x) => integer(x)?,
        None => 100,
    };
    if horizon == 0 {
        return refuse("future edits must be positive");
    }
    let fields = [
        ("runtime_noise_fraction", 0.15),
        ("cross_boundary_weight", 1.),
        ("edit_path_weight", 1.),
        ("migration_weight", 1.),
        ("merge_conflict_weight", 20.),
        ("runtime_100ms_weight", 1.),
    ];
    let mut weights = Vec::new();
    for (name, default) in fields {
        let n = field(p, name, default)?;
        if n < 0. {
            return refuse(format!("negative policy: {name}"));
        }
        weights.push(n);
    }
    if a.runtime_ms < 0. || b.runtime_ms < 0. {
        return refuse("negative runtime measurement");
    }
    let [noise, cross, edit, migration, merge, runtime]: [f64; 6] =
        weights.try_into().map_err(|_| "policy cardinality")?;
    let cost = |c: &Candidate| -> Result<f64> {
        let difference = c.runtime_ms - a.runtime_ms;
        let fraction = difference / a.runtime_ms.max(1.);
        let runtime_penalty = if fraction.abs() <= noise {
            0.
        } else {
            difference / 100.
        };
        round4(
            c.cross_edges as f64 * cross
                + c.edit_paths as f64 * edit
                + c.migration_paths as f64 * migration / horizon as f64
                + c.conflicts as f64 * merge / horizon as f64
                + runtime_penalty * runtime,
        )
    };
    let ac = cost(&a)?;
    let bc = if b.verified { Some(cost(&b)?) } else { None };
    let projected = bc.is_some_and(|x| x + 1e-9 < ac);
    let policy = obj(&[
        ("future_edits", Value::number(horizon)),
        ("runtime_noise_fraction", fvalue(noise)),
        ("cross_boundary_weight", fvalue(cross)),
        ("edit_path_weight", fvalue(edit)),
        ("migration_weight", fvalue(migration)),
        ("merge_conflict_weight", fvalue(merge)),
        ("runtime_100ms_weight", fvalue(runtime)),
    ]);
    Ok(obj(&[
        (
            "chosen",
            Value::string(if projected { "projected" } else { "original" }),
        ),
        (
            "costs",
            obj(&[
                ("original", fvalue(ac)),
                ("projected", bc.map(fvalue).unwrap_or(Value::Null)),
            ]),
        ),
        ("policy", policy),
        (
            "reason",
            Value::string(if projected {
                "Projected layout has lower explicit weighted cost under the declared horizon."
            } else {
                "Keep the verified baseline: projection has no lower weighted cost or failed verification."
            }),
        ),
    ]))
}
pub fn from_file(path: &Path) -> Result<Value> {
    let parsed = json::parse(&fs::read(path).map_err(|e| e.to_string())?)?;
    choose(&parsed)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn parse(s: &str) -> Value {
        json::parse(s.as_bytes()).unwrap()
    }
    #[test]
    fn prefers_baseline_when_projection_adds_edges() {
        let v = parse(
            r#"{"original":{"verified":true,"source_edit_paths":1,"cross_package_edges":7,"median_runtime_ms":700,"migration_paths":0,"merge_conflicts":0},"projected":{"verified":true,"source_edit_paths":1,"cross_package_edges":12,"median_runtime_ms":710,"migration_paths":7,"merge_conflicts":1},"policy":{}}"#,
        );
        let r = choose(&v).unwrap();
        assert_eq!(r.get("chosen").unwrap().str().unwrap(), "original");
        assert_eq!(
            r.get("costs").unwrap().get("original").unwrap().canonical(),
            "8.0000"
        );
        assert_eq!(
            r.get("costs")
                .unwrap()
                .get("projected")
                .unwrap()
                .canonical(),
            "13.2700"
        );
    }
    #[test]
    fn prefers_favorable_verified_projection() {
        let v = parse(
            r#"{"original":{"verified":true,"source_edit_paths":2,"cross_package_edges":15,"median_runtime_ms":700,"migration_paths":0,"merge_conflicts":0},"projected":{"verified":true,"source_edit_paths":1,"cross_package_edges":2,"median_runtime_ms":600,"migration_paths":7,"merge_conflicts":0},"policy":{}}"#,
        );
        assert_eq!(
            choose(&v).unwrap().get("chosen").unwrap().str().unwrap(),
            "projected"
        );
    }
    #[test]
    fn refuses_unverified_and_invalid_weights() {
        let v = parse(
            r#"{"original":{"verified":false,"source_edit_paths":1,"cross_package_edges":1,"median_runtime_ms":700,"migration_paths":0,"merge_conflicts":0},"projected":{"verified":true,"source_edit_paths":1,"cross_package_edges":0,"median_runtime_ms":600,"migration_paths":0,"merge_conflicts":0},"policy":{}}"#,
        );
        assert!(choose(&v).is_err());
        let mut v = v;
        if let Value::Object(ref mut m) = v {
            if let Value::Object(a) = m.get_mut("original").unwrap() {
                a.insert("verified".into(), Value::Bool(true));
            }
            m.insert("policy".into(), parse(r#"{"cross_boundary_weight":-1}"#));
        }
        assert!(choose(&v).is_err());
    }
}
