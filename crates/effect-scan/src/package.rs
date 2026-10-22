//! Package/capability, resolver and toolchain rules (EFF3xx, EFF9xx).
use crate::findings::Finding;
use crate::scan::ScanContext;

pub const RULES: &[&str] = &[];

pub fn scan(_ctx: &ScanContext) -> Vec<Finding> {
    Vec::new()
}
