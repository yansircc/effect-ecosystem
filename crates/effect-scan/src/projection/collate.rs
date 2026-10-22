//! `String.prototype.localeCompare` (ICU root collation, the Node default) for the keys the
//! projection sorts: JSON object keys, package names, file paths, rule ids.
//!
//! Exact for ASCII: whole-string primary comparison (whitespace < punctuation/symbols in CLDR root
//! order < digits < letters, case-insensitive), then the tertiary level (lowercase before
//! uppercase) on the first differing letter. Non-ASCII characters sort after ASCII letters by code
//! point: collation of accented/other scripts is not reproduced.
use std::cmp::Ordering;

/// CLDR root order of the ASCII non-alphanumerics (verified against `Intl.Collator` in Node).
const PUNCTUATION_ORDER: &str = " _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$";

fn primary(c: char) -> (u8, u32) {
    if let Some(i) = PUNCTUATION_ORDER.find(c) {
        return (0, i as u32);
    }
    if c.is_ascii_digit() {
        return (1, c as u32);
    }
    if c.is_ascii_alphabetic() {
        return (2, c.to_ascii_lowercase() as u32);
    }
    if c.is_control() {
        // Control characters are completely ignorable at every level.
        return (u8::MAX, 0);
    }
    (3, c as u32)
}

pub fn locale_compare(a: &str, b: &str) -> Ordering {
    let keys = |s: &str| -> Vec<(u8, u32)> { s.chars().map(primary).filter(|k| k.0 != u8::MAX).collect() };
    let by_primary = keys(a).cmp(&keys(b));
    if by_primary != Ordering::Equal {
        return by_primary;
    }
    // Tertiary: at the first differing letter, lowercase sorts first.
    for (x, y) in a.chars().filter(|c| !c.is_control()).zip(b.chars().filter(|c| !c.is_control())) {
        if x != y {
            return match (x.is_uppercase(), y.is_uppercase()) {
                (false, true) => Ordering::Less,
                (true, false) => Ordering::Greater,
                _ => x.cmp(&y),
            };
        }
    }
    Ordering::Equal
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_node_for_ascii_samples() {
        let sorted = [" ", "_", "-", "@", "/", "$", "0", "9", "a", "A", "b", "B", "z", "Z"];
        for pair in sorted.windows(2) {
            assert_eq!(locale_compare(pair[0], pair[1]), Ordering::Less, "{} < {}", pair[0], pair[1]);
        }
        assert_eq!(locale_compare("a-b", "ab"), Ordering::Less);
        assert_eq!(locale_compare("aB", "Ab"), Ordering::Less);
        assert_eq!(locale_compare("abc", "ab"), Ordering::Greater);
        assert_eq!(locale_compare("@effect/vitest", "effect"), Ordering::Less);
    }

    /// `EFF_COLLATE_CASES=<file of JSON string arrays in Node's order>` cross-checks against the oracle.
    #[test]
    fn oracle_order() {
        let Ok(path) = std::env::var("EFF_COLLATE_CASES") else { return };
        let list: Vec<String> = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let mut shuffled = list.clone();
        shuffled.reverse();
        shuffled.sort_by(|a, b| locale_compare(a, b));
        assert_eq!(shuffled, list);
    }
}
