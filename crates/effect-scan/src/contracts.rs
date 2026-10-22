//! Executable-edge execution contracts (EFF401) and platform constructors (EFF402).
use crate::findings::Finding;
use crate::scan::ScanContext;

pub const RULES: &[&str] = &[];

pub fn scan(_ctx: &ScanContext) -> Vec<Finding> {
    Vec::new()
}
