//! Type diagnostics from the project-local tsgo CLI (EFF500-EFF504) and strict prerequisites.
use crate::findings::Finding;
use crate::scan::ScanContext;

pub const RULES: &[&str] = &[];

pub fn scan(_ctx: &ScanContext) -> Vec<Finding> {
    Vec::new()
}
