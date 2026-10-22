//! Projections of a scan: signals, profile, evidence bundle, gate summary, compliance hash.
use crate::findings::Finding;
use crate::scan::ScanContext;
use serde_json::{Map, Value};

pub fn build(_ctx: &ScanContext, _findings: &[Finding], _warnings: &[String]) -> Map<String, Value> {
    Map::new()
}
