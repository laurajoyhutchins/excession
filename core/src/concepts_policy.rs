//! Independent, bounded prediction policies over pinned Git source changes.
//!
//! These are predictive *source companions*, never obligations, test selectors,
//! or proof of architectural semantics. Every policy sees the same chronology.
use std::collections::{BTreeMap, BTreeSet};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Policy {
    Popularity,
    Directory,
    NameCounterpart,
    ConditionalPair,
    RecentPair,
    Hybrid,
}
impl Policy {
    pub const ALL: [Self; 6] = [
        Self::Popularity,
        Self::Directory,
        Self::NameCounterpart,
        Self::ConditionalPair,
        Self::RecentPair,
        Self::Hybrid,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Popularity => "GLOBAL_POPULARITY",
            Self::Directory => "DIRECTORY_POPULARITY",
            Self::NameCounterpart => "NAME_COUNTERPART",
            Self::ConditionalPair => "CONDITIONAL_PAIR",
            Self::RecentPair => "RECENT_PAIR",
            Self::Hybrid => "HYBRID",
        }
    }
}

pub struct Model {
    files: BTreeSet<String>,
    counts: BTreeMap<String, usize>,
    pairs: BTreeMap<(String, String), usize>,
    recent: BTreeMap<(String, String), usize>,
    events: usize,
}
fn dir(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(p, _)| p)
}
fn name(path: &str) -> String {
    let leaf = path.rsplit('/').next().unwrap_or(path);
    let stem = leaf.rsplit_once('.').map_or(leaf, |(s, _)| s);
    let stem = stem
        .trim_start_matches('_')
        .strip_prefix("test_")
        .unwrap_or(stem.trim_start_matches('_'));
    let stem = stem.strip_suffix("_test").unwrap_or(stem);
    stem.bytes()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| (c as char).to_ascii_lowercase())
        .collect()
}
fn test_like(path: &str) -> bool {
    path.starts_with("tests/")
        || path.contains("/tests/")
        || path.rsplit('/').next().unwrap_or(path).starts_with("test_")
}
impl Model {
    pub fn fit(events: &[BTreeSet<String>], files: &BTreeSet<String>) -> Self {
        let mut counts = BTreeMap::new();
        let mut pairs = BTreeMap::new();
        let mut recent = BTreeMap::new();
        let recent_start = events.len().saturating_mul(3) / 4;
        for (idx, event) in events.iter().enumerate() {
            let present: Vec<_> = event.iter().filter(|p| files.contains(*p)).collect();
            for a in &present {
                *counts.entry((*a).clone()).or_insert(0) += 1;
                for b in &present {
                    if a != b {
                        let key = ((*a).clone(), (*b).clone());
                        *pairs.entry(key.clone()).or_insert(0) += 1;
                        if idx >= recent_start {
                            *recent.entry(key).or_insert(0) += 1;
                        }
                    }
                }
            }
        }
        Self {
            files: files.clone(),
            counts,
            pairs,
            recent,
            events: events.len(),
        }
    }
    fn global(&self, a: &str) -> Option<String> {
        self.counts
            .iter()
            .filter(|(p, c)| p.as_str() != a && **c != 0)
            .max_by(|(p, c), (q, d)| c.cmp(d).then_with(|| q.cmp(p)))
            .map(|(p, _)| p.clone())
    }
    fn pair(&self, a: &str, recent: bool) -> Option<String> {
        let n_a = *self.counts.get(a)?;
        if n_a < 2 {
            return None;
        }
        let mut best: Option<(u128, usize, String)> = None;
        for b in &self.files {
            if b == a {
                continue;
            }
            let key = (a.to_owned(), b.clone());
            let n = *self.pairs.get(&key).unwrap_or(&0);
            let n_b = *self.counts.get(b).unwrap_or(&0);
            if n < 2
                || n_b == 0
                || n * 2 < n_a
                || n.saturating_mul(self.events).saturating_mul(2)
                    < n_a.saturating_mul(n_b).saturating_mul(3)
            {
                continue;
            }
            let recent_count = *self.recent.get(&key).unwrap_or(&0);
            // Compare confidence with an exact rational, penalizing rare
            // companions by marginal frequency (Bayes-like shrinkage).
            let raw = ((n as u128) * (self.events as u128 + 4) * 1000)
                / ((n_a + 4) as u128 * (n_b + 2) as u128);
            let signal = if recent {
                raw + (recent_count as u128) * 40
            } else {
                raw
            };
            let score = (signal, n, b.clone());
            if best.as_ref().is_none_or(|old| {
                score.0 > old.0
                    || (score.0 == old.0
                        && (score.1 > old.1 || (score.1 == old.1 && score.2 < old.2)))
            }) {
                best = Some(score);
            }
        }
        best.map(|(_, _, path)| path)
    }
    fn directory(&self, a: &str) -> Option<String> {
        let dirname = dir(a);
        if dirname.is_empty() {
            return None;
        }
        self.counts
            .iter()
            .filter(|(b, c)| b.as_str() != a && dir(b) == dirname && **c > 0)
            .max_by(|(p, c), (q, d)| c.cmp(d).then_with(|| q.cmp(p)))
            .map(|(p, _)| p.clone())
    }
    fn counterpart(&self, a: &str) -> Option<String> {
        let stem = name(a);
        if stem.is_empty() {
            return None;
        }
        self.counts
            .iter()
            .filter(|(b, c)| b.as_str() != a && name(b) == stem && **c > 0)
            .max_by(|(p, c), (q, d)| {
                let p_test = test_like(p) != test_like(a);
                let q_test = test_like(q) != test_like(a);
                p_test
                    .cmp(&q_test)
                    .then_with(|| c.cmp(d))
                    .then_with(|| q.cmp(p))
            })
            .map(|(p, _)| p.clone())
    }
    /// One prediction at most. Equal opportunity against the global baseline.
    pub fn predict(&self, trigger: &str, policy: Policy) -> Option<String> {
        if !self.files.contains(trigger) {
            return None;
        }
        match policy {
            Policy::Popularity => self.global(trigger),
            Policy::Directory => self.directory(trigger).or_else(|| self.global(trigger)),
            Policy::NameCounterpart => self.counterpart(trigger).or_else(|| self.global(trigger)),
            Policy::ConditionalPair => self.pair(trigger, false).or_else(|| self.global(trigger)),
            Policy::RecentPair => self.pair(trigger, true).or_else(|| self.global(trigger)),
            Policy::Hybrid => self
                .counterpart(trigger)
                .or_else(|| self.pair(trigger, true))
                .or_else(|| self.directory(trigger))
                .or_else(|| self.global(trigger)),
        }
    }
}

#[derive(Default, Clone, Debug)]
pub struct Score {
    pub opportunities: usize,
    pub model_hits: usize,
    pub popularity_hits: usize,
    pub directory_hits: usize,
    pub model_only: usize,
    pub popularity_only: usize,
    pub differences: usize,
    pub missing_prediction: usize,
}
impl Score {
    pub fn delta(&self) -> i64 {
        self.model_only as i64 - self.popularity_only as i64
    }
}
pub fn score(model: &Model, policy: Policy, events: &[BTreeSet<String>]) -> Score {
    let mut result = Score::default();
    // Cache predictions once per source and policy, not per occurrence.
    let mut cache = BTreeMap::<String, (Option<String>, Option<String>, Option<String>)>::new();
    for event in events {
        for trigger in event.iter().filter(|p| model.files.contains(*p)) {
            let (pick, base, local) = cache.entry(trigger.clone()).or_insert_with(|| {
                (
                    model.predict(trigger, policy),
                    model.predict(trigger, Policy::Popularity),
                    model.predict(trigger, Policy::Directory),
                )
            });
            if base.is_none() {
                result.missing_prediction += 1;
                continue;
            }
            result.opportunities += 1;
            let model_hit = pick.as_ref().is_some_and(|p| event.contains(p));
            let base_hit = base.as_ref().is_some_and(|p| event.contains(p));
            let local_hit = local.as_ref().is_some_and(|p| event.contains(p));
            result.model_hits += usize::from(model_hit);
            result.popularity_hits += usize::from(base_hit);
            result.directory_hits += usize::from(local_hit);
            result.model_only += usize::from(model_hit && !base_hit);
            result.popularity_only += usize::from(!model_hit && base_hit);
            result.differences += usize::from(pick != base);
        }
    }
    result
}

/// Inner chronological validation chooses a policy before the external future
/// has been examined. Popularity wins ties. Require an observable positive
/// advantage, rather than selecting a complicated model by default.
pub fn choose(
    fit: &[BTreeSet<String>],
    calibration: &[BTreeSet<String>],
    historical_files: &BTreeSet<String>,
) -> (Policy, Vec<(Policy, Score)>) {
    let model = Model::fit(fit, historical_files);
    let results: Vec<_> = Policy::ALL
        .iter()
        .map(|&p| (p, score(&model, p, calibration)))
        .collect();
    let mut best = Policy::Popularity;
    let mut advantage = 0_i64;
    for (policy, stats) in &results {
        if stats.opportunities >= 8 && stats.delta() > advantage {
            advantage = stats.delta();
            best = *policy;
        }
    }
    (best, results)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(s: &[&str]) -> BTreeSet<String> {
        s.iter().map(|x| x.to_string()).collect()
    }
    #[test]
    fn different_signals_outperform_global_popularity_on_frozen_cases() {
        let mut training = Vec::new();
        for _ in 0..7 {
            training.push(event(&["src/api.py", "src/handler.py"]));
        }
        for _ in 0..10 {
            training.push(event(&["src/popular.py"]));
        }
        for _ in 0..3 {
            training.push(event(&["src/api.py"]));
        }
        let files = event(&[
            "src/api.py",
            "src/handler.py",
            "src/popular.py",
            "tests/test_api.py",
        ]);
        let m = Model::fit(&training, &files);
        let tests = vec![
            event(&["src/api.py", "src/handler.py"]),
            event(&["src/api.py", "src/handler.py"]),
        ];
        let tested = score(&m, Policy::ConditionalPair, &tests);
        assert_eq!(tested.model_hits, 4);
        // A globally popular file ties the trigger count, so lexical
        // tie-breaking correctly finds the reciprocal source twice.
        assert_eq!(tested.popularity_hits, 2);
        assert_eq!(tested.model_only, 2);
        assert_eq!(tested.opportunities, 4);
        assert_eq!(
            m.predict("src/api.py", Policy::NameCounterpart),
            Some("src/popular.py".into())
        );
    }
    #[test]
    fn exact_source_test_stems_survive_directory_difference() {
        let files = event(&["src/client.py", "tests/test_client.py", "src/util.py"]);
        let training = vec![
            event(&["src/client.py"]),
            event(&["tests/test_client.py"]),
            event(&["src/util.py"]),
            event(&["src/util.py"]),
        ];
        let model = Model::fit(&training, &files);
        assert_eq!(
            model.predict("src/client.py", Policy::NameCounterpart),
            Some("tests/test_client.py".into())
        );
    }
    #[test]
    fn calibration_uses_no_external_holdout() {
        let files = event(&["src/api.py", "src/handler.py", "src/popular.py"]);
        let fit = (0..10)
            .map(|_| event(&["src/api.py", "src/handler.py"]))
            .chain((0..11).map(|_| event(&["src/popular.py"])))
            .collect::<Vec<_>>();
        let cal = vec![event(&["src/api.py", "src/handler.py"]); 5];
        let (policy, assessed) = choose(&fit, &cal, &files);
        assert_ne!(policy, Policy::Popularity);
        assert_eq!(assessed.len(), 6);
        let empty: Vec<BTreeSet<String>> = Vec::new();
        assert_eq!(choose(&fit, &empty, &files).0, Policy::Popularity);
    }
    #[test]
    fn never_forecasts_unseen_or_same_source() {
        let paths = event(&["src/a.py", "src/b.py"]);
        let model = Model::fit(&[event(&["src/a.py", "src/b.py"])], &paths);
        assert!(model.predict("src/new.py", Policy::Hybrid).is_none());
        for p in Policy::ALL {
            assert_ne!(model.predict("src/a.py", p), Some("src/a.py".into()));
        }
    }
}
