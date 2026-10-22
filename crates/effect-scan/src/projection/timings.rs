//! `--timings`: wall-clock per stage, in whole milliseconds, plus the total.
use super::collate::locale_compare;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::time::Instant;

pub struct Timings {
    start: Instant,
    last_mark: Instant,
    stages: BTreeMap<String, i64>,
}

impl Timings {
    pub fn new() -> Self {
        let now = Instant::now();
        Self { start: now, last_mark: now, stages: BTreeMap::new() }
    }

    fn add(&mut self, name: &str, ms: f64) {
        let entry = self.stages.entry(name.to_string()).or_insert(0);
        *entry = (*entry as f64 + ms).round() as i64;
    }

    /// Attributes the time since the previous mark (or the start) to stage `name`. Marks are plain
    /// statements between pipeline steps, so instrumenting a step never rewrites the step.
    pub fn mark(&mut self, name: &str) {
        let now = Instant::now();
        let ms = now.duration_since(self.last_mark).as_secs_f64() * 1000.0;
        self.last_mark = now;
        self.add(name, ms);
    }

    /// Runs `f`, adding its duration to stage `name` (rounded per accumulation, as the oracle does).
    pub fn measure<T>(&mut self, name: &str, f: impl FnOnce() -> T) -> T {
        let before = Instant::now();
        let out = f();
        self.add(name, before.elapsed().as_secs_f64() * 1000.0);
        self.last_mark = Instant::now();
        out
    }

    pub fn snapshot(&self) -> Value {
        let mut ordered: Vec<(&String, &i64)> = self.stages.iter().collect();
        ordered.sort_by(|a, b| locale_compare(a.0, b.0));
        let stages: Map<String, Value> = ordered.into_iter().map(|(k, v)| (k.clone(), json!(v))).collect();
        json!({ "totalMs": self.start.elapsed().as_secs_f64().mul_add(1000.0, 0.0).round() as i64, "stages": stages })
    }
}
