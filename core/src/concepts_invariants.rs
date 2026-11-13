//! Source-bound historical change-companion hypotheses for ontology review.
//! Co-change is an observation, not proof of a behavioral invariant.
use crate::git;
use crate::json::{Value, obj};
use crate::{Result, refuse};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const MAX_FILES: usize = 24;
const MAX_RULES: usize = 64;
const MAX_DIFF: usize = 4 * 1024 * 1024;
const WIRE: &str = "excession.concepts-temporal-obligations.v1";
const SCHEMA: &str = include_str!("../../contracts/concepts/temporal-obligations.v1.schema.json");

#[derive(Clone, Debug)]
pub struct Settings {
    pub train: usize,
    pub holdout: usize,
    pub support: usize,
    pub confidence_percent: usize,
    pub prefix: String,
}
impl Settings {
    fn check(&self, backtest: bool) -> Result<()> {
        if !(8..=256).contains(&self.train)
            || (backtest && !(1..=100).contains(&self.holdout))
            || (!backtest && self.holdout != 0)
            || !(2..=32).contains(&self.support)
            || !(50..=100).contains(&self.confidence_percent)
        {
            return refuse(
                "invalid temporal settings (train 8..256, backtest holdout 1..100, support 2..32, confidence 50..100)",
            );
        }
        if self.prefix != "." {
            git::validate_path(&self.prefix)?;
        }
        Ok(())
    }
}

#[derive(Clone)]
struct Event {
    commit: String,
    paths: BTreeSet<String>,
    excluded: Option<&'static str>,
}
fn is_source(path: &str) -> bool {
    [
        ".rs", ".py", ".js", ".ts", ".jsx", ".tsx", ".go", ".java", ".c", ".h", ".cpp", ".cc",
        ".cs", ".rb", ".swift",
    ]
    .iter()
    .any(|suffix| path.ends_with(suffix))
}
fn in_scope(path: &str, prefix: &str) -> bool {
    prefix == "."
        || path
            .strip_prefix(prefix)
            .is_some_and(|suffix| suffix.starts_with('/'))
}
fn modified(repo: &Path, base: &str, head: &str, prefix: &str) -> Result<BTreeSet<String>> {
    git::oid(base)?;
    git::oid(head)?;
    // No renames; modifications only. Added/deleted files cannot generate
    // artificial co-change rules, and a path is an exact Git identity.
    let output = git::command(
        repo,
        &[
            "diff",
            "--no-renames",
            "--diff-filter=M",
            "--name-only",
            "-z",
            base,
            head,
            "--",
        ],
    )?;
    if output.len() > MAX_DIFF || (!output.is_empty() && output.last() != Some(&0)) {
        return refuse("unbounded or malformed Git diff output");
    }
    let mut result = BTreeSet::new();
    for part in output.split(|byte| *byte == 0).filter(|p| !p.is_empty()) {
        let path = std::str::from_utf8(part).map_err(|_| "historical path is not UTF-8")?;
        git::validate_path(path)?;
        if is_source(path) && in_scope(path, prefix) {
            result.insert(path.to_owned());
        }
    }
    Ok(result)
}
fn history(repo: &Path, rev: &str, count: usize, prefix: &str) -> Result<Vec<Event>> {
    git::verify_repo(repo)?;
    let head = git::rev(repo, rev)?;
    if git::text(repo, &["cat-file", "-t", &head])? != "commit" {
        return refuse("historical anchor must be a commit");
    }
    let cap = (count + 1).to_string();
    let mut commits: Vec<String> = git::text(
        repo,
        &["rev-list", "--first-parent", "--max-count", &cap, &head],
    )?
    .lines()
    .map(str::to_owned)
    .collect();
    if commits.len() != count + 1 {
        return refuse("insufficient first-parent history for the requested training and holdout");
    }
    for id in &commits {
        git::oid(id)?;
    }
    commits.reverse();
    let mut result = Vec::new();
    for pair in commits.windows(2) {
        let commit = &pair[1];
        let parents = git::text(repo, &["rev-list", "--parents", "-n", "1", commit])?;
        let pieces: Vec<_> = parents.split_whitespace().collect();
        if pieces.first() != Some(&commit.as_str()) || pieces.get(1) != Some(&pair[0].as_str()) {
            return refuse("historical event violates the first-parent chain");
        }
        let merge = pieces.len() > 2;
        let paths = if merge {
            BTreeSet::new()
        } else {
            modified(repo, &pair[0], commit, prefix)?
        };
        let excluded = if merge {
            Some("MERGE")
        } else if paths.is_empty() {
            Some("EMPTY_SCOPE")
        } else if paths.len() > MAX_FILES {
            Some("BULK_CHANGE")
        } else {
            None
        };
        result.push(Event {
            commit: commit.clone(),
            paths,
            excluded,
        });
    }
    Ok(result)
}
fn event_summary(events: &[Event]) -> Value {
    let mut gaps = BTreeMap::<&str, usize>::new();
    for event in events {
        if let Some(code) = event.excluded {
            *gaps.entry(code).or_default() += 1;
        }
    }
    obj(&[
        ("events", Value::number(events.len())),
        (
            "eligible",
            Value::number(events.iter().filter(|e| e.excluded.is_none()).count()),
        ),
        (
            "gaps",
            Value::array(gaps.into_iter().map(|(code, n)| {
                obj(&[("code", Value::string(code)), ("count", Value::number(n))])
            })),
        ),
    ])
}
#[derive(Clone)]
struct Rule {
    trigger: String,
    companion: String,
    trigger_events: usize,
    companion_events: usize,
    cochanges: Vec<String>,
}
fn compare(a: &Rule, b: &Rule) -> std::cmp::Ordering {
    // Prefer conditional confidence, then lift, then support and lexical order.
    let left = (a.cochanges.len() as u128) * (b.trigger_events as u128);
    let right = (b.cochanges.len() as u128) * (a.trigger_events as u128);
    right
        .cmp(&left)
        .then_with(|| {
            let l = (a.cochanges.len() as u128)
                * (b.trigger_events as u128)
                * (b.companion_events as u128);
            let r = (b.cochanges.len() as u128)
                * (a.trigger_events as u128)
                * (a.companion_events as u128);
            r.cmp(&l)
        })
        .then_with(|| b.cochanges.len().cmp(&a.cochanges.len()))
        .then_with(|| a.companion.cmp(&b.companion))
}
fn train(
    events: &[Event],
    blobs: &BTreeMap<String, String>,
    settings: &Settings,
) -> (Vec<Rule>, BTreeMap<String, usize>, usize, usize) {
    let mut counts = BTreeMap::<String, usize>::new();
    let mut both = BTreeMap::<(String, String), Vec<String>>::new();
    let mut eligible = 0_usize;
    for event in events.iter().filter(|e| e.excluded.is_none()) {
        eligible += 1;
        let paths: Vec<_> = event
            .paths
            .iter()
            .filter(|p| blobs.contains_key(*p))
            .collect();
        for a in &paths {
            *counts.entry((*a).clone()).or_default() += 1;
            for b in &paths {
                if a != b {
                    both.entry(((*a).clone(), (*b).clone()))
                        .or_default()
                        .push(event.commit.clone());
                }
            }
        }
    }
    let mut best = BTreeMap::<String, Rule>::new();
    for ((trigger, companion), witnesses) in both {
        let a = counts[&trigger];
        let b = counts[&companion];
        let n = witnesses.len();
        if n < settings.support || a < 3
            || n * 100 < settings.confidence_percent * a
            // Lift >= 1.5 above independent base-rate co-occurrence.
            || 2 * n * eligible < 3 * a * b
        {
            continue;
        }
        let candidate = Rule {
            trigger: trigger.clone(),
            companion,
            trigger_events: a,
            companion_events: b,
            cochanges: witnesses,
        };
        match best.get_mut(&trigger) {
            Some(previous) if compare(&candidate, previous).is_lt() => *previous = candidate,
            None => {
                best.insert(trigger, candidate);
            }
            _ => {}
        }
    }
    let mut rules: Vec<_> = best.into_values().collect();
    rules.sort_by(|a, b| compare(a, b).then_with(|| a.trigger.cmp(&b.trigger)));
    let dropped = rules.len().saturating_sub(MAX_RULES);
    rules.truncate(MAX_RULES);
    (rules, counts, eligible, dropped)
}
fn popular(counts: &BTreeMap<String, usize>, skip: &str) -> Option<String> {
    counts
        .iter()
        .filter(|(p, _)| p.as_str() != skip)
        .max_by(|(pa, ca), (pb, cb)| ca.cmp(cb).then_with(|| pb.cmp(pa)))
        .map(|(p, _)| p.clone())
}
fn report(
    repo: &Path,
    rev: &str,
    settings: &Settings,
    backtest: bool,
    candidate: Option<&str>,
) -> Result<Value> {
    settings.check(backtest)?;
    if backtest && candidate.is_some() {
        return refuse("backtest and prospective candidate are exclusive");
    }
    let events = history(
        repo,
        rev,
        settings.train + settings.holdout,
        &settings.prefix,
    )?;
    let (training, holdout) = events.split_at(settings.train);
    let cutoff = &training.last().ok_or("empty training slice")?.commit;
    let cutoff_tree = git::rev(repo, &format!("{cutoff}^{{tree}}"))?;
    let blobs: BTreeMap<String, String> = git::entries(repo, cutoff)?
        .into_iter()
        .filter(|e| e.kind == "blob" && is_source(&e.path) && in_scope(&e.path, &settings.prefix))
        .map(|e| (e.path, e.oid))
        .collect();
    let (rules, counts, eligible, dropped) = train(training, &blobs, settings);
    let proposals = rules
        .iter()
        .map(|rule| {
            obj(&[
                ("predicate", Value::string("HISTORICAL_CHANGE_COMPANION")),
                ("status", Value::string("PROPOSED_UNTESTED")),
                ("trigger_path", Value::string(&rule.trigger)),
                ("trigger_blob", Value::string(&blobs[&rule.trigger])),
                ("companion_path", Value::string(&rule.companion)),
                ("companion_blob", Value::string(&blobs[&rule.companion])),
                ("source_commit", Value::string(cutoff)),
                ("source_tree", Value::string(&cutoff_tree)),
                ("trigger_events", Value::number(rule.trigger_events)),
                ("companion_events", Value::number(rule.companion_events)),
                ("cochange_events", Value::number(rule.cochanges.len())),
                ("total_training_events", Value::number(eligible)),
                (
                    "witness_commits",
                    Value::array(rule.cochanges.iter().map(Value::string)),
                ),
                (
                    "baseline_companion",
                    popular(&counts, &rule.trigger).map_or(Value::Null, Value::string),
                ),
                ("semantic_validity", Value::string("NOT_EVALUATED")),
            ])
        })
        .collect::<Vec<_>>();
    let future = candidate.map(|r| git::rev(repo, r)).transpose()?;
    let prospective = if let Some(head) = &future {
        if head == cutoff {
            return refuse("a prospective candidate must differ from training cutoff");
        }
        git::text(repo, &["merge-base", "--is-ancestor", cutoff, head])?;
        let paths = modified(repo, cutoff, head, &settings.prefix)?;
        if paths.len() > MAX_FILES {
            return refuse("prospective change exceeds 24-file comparison bound");
        }
        Some(Event {
            commit: head.clone(),
            paths,
            excluded: None,
        })
    } else {
        None
    };
    let score_events: Vec<&Event> = match &prospective {
        Some(event) => vec![event],
        None => holdout
            .iter()
            .filter(|event| event.excluded.is_none())
            .collect(),
    };
    let mut exposed = 0_usize;
    let mut matches = 0_usize;
    let mut base_matches = 0_usize;
    let mut model_only_hits = 0_usize;
    let mut baseline_only_hits = 0_usize;
    let mut observations = Vec::new();
    for event in score_events {
        for rule in &rules {
            if !event.paths.contains(&rule.trigger) {
                continue;
            }
            let found = event.paths.contains(&rule.companion);
            let baseline = popular(&counts, &rule.trigger);
            let base_found = baseline.as_ref().is_some_and(|p| event.paths.contains(p));
            exposed += 1;
            matches += usize::from(found);
            base_matches += usize::from(base_found);
            model_only_hits += usize::from(found && !base_found);
            baseline_only_hits += usize::from(!found && base_found);
            observations.push(obj(&[
                ("commit", Value::string(&event.commit)),
                ("trigger_path", Value::string(&rule.trigger)),
                ("predicted_companion", Value::string(&rule.companion)),
                ("companion_modified", Value::Bool(found)),
                (
                    "baseline_companion",
                    baseline.map_or(Value::Null, Value::string),
                ),
                ("baseline_companion_modified", Value::Bool(base_found)),
                (
                    "status",
                    Value::string(if found {
                        "MATCHED"
                    } else {
                        "NOT_OBSERVED_NOT_A_PROVEN_BUG"
                    }),
                ),
            ]));
        }
    }
    let mut value = obj(&[
        ("schema", Value::string(WIRE)),
        (
            "status",
            Value::string("UNVERIFIED_HISTORICAL_ASSOCIATIONS"),
        ),
        (
            "mode",
            Value::string(if backtest {
                "BACKTEST"
            } else if future.is_some() {
                "PROSPECTIVE"
            } else {
                "DISCOVERY"
            }),
        ),
        ("requested_commit", Value::string(git::rev(repo, rev)?)),
        ("training_cutoff", Value::string(cutoff)),
        ("training_tree", Value::string(&cutoff_tree)),
        (
            "prospective_commit",
            future.map_or(Value::Null, Value::string),
        ),
        ("prefix", Value::string(&settings.prefix)),
        ("training", event_summary(training)),
        ("holdout", event_summary(holdout)),
        (
            "parameters",
            obj(&[
                ("train", Value::number(settings.train)),
                ("holdout", Value::number(settings.holdout)),
                ("support", Value::number(settings.support)),
                (
                    "confidence_percent",
                    Value::number(settings.confidence_percent),
                ),
            ]),
        ),
        ("candidates", Value::array(proposals)),
        ("candidate_count", Value::number(rules.len())),
        ("omitted_candidates", Value::number(dropped)),
        ("enumeration_complete", Value::Bool(dropped == 0)),
        ("exposures", Value::number(exposed)),
        ("model_matches", Value::number(matches)),
        ("baseline_matches", Value::number(base_matches)),
        ("model_only_hits", Value::number(model_only_hits)),
        ("baseline_only_hits", Value::number(baseline_only_hits)),
        ("observations", Value::array(observations)),
        (
            "model_precision",
            if exposed == 0 {
                Value::Null
            } else {
                Value::string(format!("{matches}/{exposed}"))
            },
        ),
        (
            "baseline_precision",
            if exposed == 0 {
                Value::Null
            } else {
                Value::string(format!("{base_matches}/{exposed}"))
            },
        ),
        ("behavioral_validity", Value::string("NOT_EVALUATED")),
        ("safe_to_skip_unlisted_tests", Value::Bool(false)),
    ]);
    let id = value.sha256();
    value
        .entries_mut()?
        .insert("report_id".into(), Value::string(id));
    let schema: serde_json::Value = serde_json::from_str(SCHEMA).map_err(|e| e.to_string())?;
    let instance: serde_json::Value =
        serde_json::from_str(&value.canonical()).map_err(|e| e.to_string())?;
    let validator = jsonschema::draft202012::new(&schema)
        .map_err(|e| format!("temporal schema compilation: {e}"))?;
    if let Err(error) = validator.validate(&instance) {
        return refuse(format!("invalid temporal report: {error}"));
    }
    Ok(value)
}
pub fn propose(
    repo: &Path,
    rev: &str,
    settings: &Settings,
    candidate: Option<&str>,
) -> Result<Value> {
    report(repo, rev, settings, false, candidate)
}
pub fn backtest(repo: &Path, rev: &str, settings: &Settings) -> Result<Value> {
    report(repo, rev, settings, true, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    fn run(repo: &Path, args: &[&str]) {
        git::text(repo, args).unwrap();
    }
    fn commit(repo: &Path, paths: &[&str], step: usize) {
        for file in paths {
            fs::write(repo.join(file), format!("{file}: {step}\n")).unwrap();
        }
        run(repo, &["add", "."]);
        run(
            repo,
            &[
                "-c",
                "user.name=Temporal test",
                "-c",
                "user.email=temporal@example.invalid",
                "commit",
                "-qm",
                "test fixture",
            ],
        );
    }
    fn fixture() -> (std::path::PathBuf, String, String) {
        let root = std::env::temp_dir().join(format!(
            "excession-temporal-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        run(&root, &["init", "-q"]);
        commit(&root, &["alpha.py", "beta.py", "popular.py", "other.py"], 0);
        for step in 1..=20 {
            let paths: &[&str] = match step {
                1..=6 => &["alpha.py", "beta.py"],
                7 => &["alpha.py"],
                8 => &["beta.py"],
                9..=18 => &["popular.py"],
                _ => &["other.py"],
            };
            commit(&root, paths, step);
        }
        let cutoff = git::rev(&root, "HEAD").unwrap();
        commit(&root, &["alpha.py", "beta.py"], 21);
        commit(&root, &["alpha.py"], 22);
        commit(&root, &["other.py"], 23);
        let latest = git::rev(&root, "HEAD").unwrap();
        (root, cutoff, latest)
    }
    #[test]
    fn history_predicts_later_change_better_than_popularity_in_controlled_fixture() {
        let (root, cutoff, latest) = fixture();
        let config = Settings {
            train: 20,
            holdout: 3,
            support: 3,
            confidence_percent: 70,
            prefix: ".".into(),
        };
        let answer = backtest(&root, &latest, &config).unwrap();
        assert_eq!(
            answer.get("training_cutoff").unwrap().str().unwrap(),
            cutoff
        );
        assert_eq!(answer.get("candidate_count").unwrap().integer().unwrap(), 2);
        assert_eq!(answer.get("exposures").unwrap().integer().unwrap(), 3);
        assert_eq!(answer.get("model_matches").unwrap().integer().unwrap(), 2);
        assert_eq!(
            answer.get("baseline_matches").unwrap().integer().unwrap(),
            0
        );
        assert_eq!(
            answer.get("behavioral_validity").unwrap().str().unwrap(),
            "NOT_EVALUATED"
        );
        assert_eq!(answer.get("model_only_hits").unwrap().integer().unwrap(), 2);
        assert_eq!(
            answer.get("baseline_only_hits").unwrap().integer().unwrap(),
            0
        );
        assert!(
            answer
                .get("observations")
                .unwrap()
                .array_ref()
                .unwrap()
                .iter()
                .any(|item| item.get("status").unwrap().str().unwrap()
                    == "NOT_OBSERVED_NOT_A_PROVEN_BUG")
        );
        let prospect = Settings {
            holdout: 0,
            ..config
        };
        let prospective = propose(&root, &cutoff, &prospect, Some(&latest)).unwrap();
        assert_eq!(
            prospective.get("training_cutoff").unwrap().str().unwrap(),
            cutoff
        );
        assert_eq!(
            prospective.get("mode").unwrap().str().unwrap(),
            "PROSPECTIVE"
        );
        assert_eq!(
            answer.get("candidates").unwrap(),
            prospective.get("candidates").unwrap()
        );
        assert!(
            !prospective
                .get("safe_to_skip_unlisted_tests")
                .unwrap()
                .bool()
                .unwrap()
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn settings_refuse_unbounded_or_unsafe_requests() {
        assert!(
            Settings {
                train: 7,
                holdout: 0,
                support: 2,
                confidence_percent: 60,
                prefix: ".".into()
            }
            .check(false)
            .is_err()
        );
        assert!(
            Settings {
                train: 20,
                holdout: 1,
                support: 2,
                confidence_percent: 60,
                prefix: "../escape".into()
            }
            .check(true)
            .is_err()
        );
    }
}
