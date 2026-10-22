//! node-semver 7.x semantics (`satisfies`, `validRange`, `subset`) with `includePrerelease: true`,
//! the only mode the scanner uses. Ported from node-semver's `classes/{semver,range,comparator}`
//! and `ranges/subset` over the same token grammar (`internal/re`), so range text is interpreted
//! by one grammar in both scanners. The `semver` crate is Cargo-flavoured (comma-joined
//! comparators, no `||`, no x-ranges, no hyphen ranges) and cannot express these ranges.
use regex::{Captures, Regex};
use std::cmp::Ordering;
use std::sync::LazyLock;

const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;
const MAX_LENGTH: usize = 256;

struct Grammar {
    full: Regex,
    comparator: Regex,
    xrange: Regex,
    tilde: Regex,
    caret: Regex,
    tilde_trim: Regex,
    caret_trim: Regex,
    comparator_trim: Regex,
    hyphen: Regex,
    star: Regex,
    gte0pre: Regex,
    build: Regex,
}

static G: LazyLock<Grammar> = LazyLock::new(|| {
    let num = "0|[1-9][0-9]*";
    let nonnum = "[0-9]*[a-zA-Z-][a-zA-Z0-9-]*";
    let main = format!("({num})\\.({num})\\.({num})");
    let pre_id = format!("(?:{nonnum}|{num})");
    let prerelease = format!("(?:-({pre_id}(?:\\.{pre_id})*))");
    let build = "(?:\\+([a-zA-Z0-9-]+(?:\\.[a-zA-Z0-9-]+)*))".to_string();
    let fullplain = format!("v?{main}{prerelease}?{build}?");
    let gtlt = "((?:<|>)?=?)";
    let xid = format!("{num}|x|X|\\*");
    let xrangeplain = format!("[v=\\s]*({xid})(?:\\.({xid})(?:\\.({xid})(?:{prerelease})?{build}?)?)?");
    let main_loose = "([0-9]+)\\.([0-9]+)\\.([0-9]+)".to_string();
    let pre_id_loose = format!("(?:{nonnum}|[0-9]+)");
    let prerelease_loose = format!("(?:-?({pre_id_loose}(?:\\.{pre_id_loose})*))");
    let looseplain = format!("[v=\\s]*{main_loose}{prerelease_loose}?{build}?");
    let re = |s: String| Regex::new(&s).unwrap_or_else(|e| panic!("semver grammar: {e}"));
    Grammar {
        full: re(format!("^{fullplain}$")),
        comparator: re(format!("^{gtlt}\\s*({fullplain})$|^$")),
        xrange: re(format!("^{gtlt}\\s*{xrangeplain}$")),
        tilde: re(format!("^(?:~>?){xrangeplain}$")),
        caret: re(format!("^(?:\\^){xrangeplain}$")),
        tilde_trim: re("(\\s*)(?:~>?)\\s+".into()),
        caret_trim: re("(\\s*)(?:\\^)\\s+".into()),
        comparator_trim: re(format!("(\\s*){gtlt}\\s*({looseplain}|{xrangeplain})")),
        hyphen: re(format!("^\\s*({xrangeplain})\\s+-\\s+({xrangeplain})\\s*$")),
        star: re("(<|>)?=?\\s*\\*".into()),
        gte0pre: re("^\\s*>=\\s*0\\.0\\.0-0\\s*$".into()),
        build: re(build),
    }
});

fn group<'a>(c: &Captures<'a>, i: usize) -> &'a str {
    c.get(i).map_or("", |m| m.as_str())
}

#[derive(Debug, Clone, PartialEq)]
enum Id {
    Num(u64),
    Str(String),
}

impl Id {
    fn text(&self) -> String {
        match self {
            Id::Num(n) => n.to_string(),
            Id::Str(s) => s.clone(),
        }
    }
}

fn compare_identifiers(a: &Id, b: &Id) -> Ordering {
    if let (Id::Num(x), Id::Num(y)) = (a, b) {
        return x.cmp(y);
    }
    let (at, bt) = (a.text(), b.text());
    let numeric = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit());
    let (an, bn) = (numeric(&at), numeric(&bt));
    if an && bn {
        let (x, y) = (at.parse::<f64>().unwrap_or(f64::MAX), bt.parse::<f64>().unwrap_or(f64::MAX));
        return x.partial_cmp(&y).unwrap_or(Ordering::Equal);
    }
    match (an, bn) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        _ => at.cmp(&bt),
    }
}

#[derive(Debug, Clone)]
pub struct SemVer {
    major: u64,
    minor: u64,
    patch: u64,
    prerelease: Vec<Id>,
    version: String,
}

impl SemVer {
    pub fn parse(text: &str) -> Option<SemVer> {
        if text.len() > MAX_LENGTH {
            return None;
        }
        let caps = G.full.captures(text.trim())?;
        let part = |i: usize| group(&caps, i).parse::<u64>().ok().filter(|n| *n <= MAX_SAFE_INTEGER);
        let (major, minor, patch) = (part(1)?, part(2)?, part(3)?);
        let prerelease: Vec<Id> = match caps.get(4) {
            Some(m) => m
                .as_str()
                .split('.')
                .map(|id| match id.parse::<u64>() {
                    Ok(n) if id.bytes().all(|c| c.is_ascii_digit()) && n < MAX_SAFE_INTEGER => Id::Num(n),
                    _ => Id::Str(id.to_string()),
                })
                .collect(),
            None => Vec::new(),
        };
        let mut version = format!("{major}.{minor}.{patch}");
        if !prerelease.is_empty() {
            version.push('-');
            version.push_str(&prerelease.iter().map(Id::text).collect::<Vec<_>>().join("."));
        }
        Some(SemVer { major, minor, patch, prerelease, version })
    }

    fn compare(&self, other: &SemVer) -> Ordering {
        if self.version == other.version {
            return Ordering::Equal;
        }
        let main = (self.major, self.minor, self.patch).cmp(&(other.major, other.minor, other.patch));
        if main != Ordering::Equal {
            return main;
        }
        match (self.prerelease.is_empty(), other.prerelease.is_empty()) {
            (false, true) => return Ordering::Less,
            (true, false) => return Ordering::Greater,
            (true, true) => return Ordering::Equal,
            _ => {}
        }
        for i in 0.. {
            match (self.prerelease.get(i), other.prerelease.get(i)) {
                (None, None) => return Ordering::Equal,
                (Some(_), None) => return Ordering::Greater,
                (None, Some(_)) => return Ordering::Less,
                (Some(a), Some(b)) if a == b => continue,
                (Some(a), Some(b)) => return compare_identifiers(a, b),
            }
        }
        Ordering::Equal
    }
}

#[derive(Debug, Clone)]
struct Comparator {
    operator: String,
    /// `None` is node-semver's `ANY`.
    semver: Option<SemVer>,
    value: String,
}

impl Comparator {
    fn new(text: &str) -> Option<Comparator> {
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let caps = G.comparator.captures(&text)?;
        let mut operator = caps.get(1).map_or("", |m| m.as_str()).to_string();
        if operator == "=" {
            operator.clear();
        }
        let version = caps.get(2).map_or("", |m| m.as_str());
        if version.is_empty() {
            return Some(Comparator { operator, semver: None, value: String::new() });
        }
        let semver = SemVer::parse(version)?;
        let value = format!("{operator}{}", semver.version);
        Some(Comparator { operator, semver: Some(semver), value })
    }

    fn test(&self, version: &SemVer) -> bool {
        let Some(own) = &self.semver else { return true };
        let ord = version.compare(own);
        match self.operator.as_str() {
            "" => ord == Ordering::Equal,
            ">" => ord == Ordering::Greater,
            ">=" => ord != Ordering::Less,
            "<" => ord == Ordering::Less,
            "<=" => ord != Ordering::Greater,
            _ => false,
        }
    }

    fn is_null_set(&self) -> bool {
        self.value == "<0.0.0-0"
    }
}

fn is_x(id: &str) -> bool {
    id.is_empty() || id.eq_ignore_ascii_case("x") || id == "*"
}

fn inc(n: &str) -> String {
    (n.parse::<u64>().unwrap_or(u64::MAX).saturating_add(1)).to_string()
}

fn replace_tilde(comp: &str) -> String {
    G.tilde
        .replace(comp, |c: &Captures| {
            let (major, minor, patch, pre) = (group(c, 1), group(c, 2), group(c, 3), group(c, 4));
            if is_x(major) {
                String::new()
            } else if is_x(minor) {
                format!(">={major}.0.0-0 <{}.0.0-0", inc(major))
            } else if is_x(patch) {
                format!(">={major}.{minor}.0-0 <{major}.{}.0-0", inc(minor))
            } else if !pre.is_empty() {
                format!(">={major}.{minor}.{patch}-{pre} <{major}.{}.0-0", inc(minor))
            } else {
                format!(">={major}.{minor}.{patch} <{major}.{}.0-0", inc(minor))
            }
        })
        .into_owned()
}

fn replace_caret(comp: &str) -> String {
    G.caret
        .replace(comp, |c: &Captures| {
            let (major, minor, patch, pre) = (group(c, 1), group(c, 2), group(c, 3), group(c, 4));
            if is_x(major) {
                String::new()
            } else if is_x(minor) {
                format!(">={major}.0.0-0 <{}.0.0-0", inc(major))
            } else if is_x(patch) {
                if major == "0" {
                    format!(">={major}.{minor}.0-0 <{major}.{}.0-0", inc(minor))
                } else {
                    format!(">={major}.{minor}.0-0 <{}.0.0-0", inc(major))
                }
            } else {
                let lower = if pre.is_empty() { format!(">={major}.{minor}.{patch}") } else { format!(">={major}.{minor}.{patch}-{pre}") };
                if major == "0" {
                    if minor == "0" {
                        format!("{lower} <{major}.{minor}.{}-0", inc(patch))
                    } else {
                        format!("{lower} <{major}.{}.0-0", inc(minor))
                    }
                } else {
                    format!("{lower} <{}.0.0-0", inc(major))
                }
            }
        })
        .into_owned()
}

/// JS `s.trim().split(/\s+/)`: an empty string yields one empty word.
fn words(s: &str) -> Vec<&str> {
    let t = s.trim();
    if t.is_empty() {
        vec![""]
    } else {
        t.split_whitespace().collect()
    }
}

fn replace_xrange(comp: &str) -> String {
    let comp = comp.trim();
    G.xrange
        .replace(comp, |c: &Captures| {
            let matched = group(c, 0).to_string();
            let mut gtlt = group(c, 1).to_string();
            let (mut major, mut minor, mut patch) = (group(c, 2).to_string(), group(c, 3).to_string(), group(c, 4).to_string());
            if (is_x(&major) && !is_x(&minor)) || (is_x(&minor) && !patch.is_empty() && !is_x(&patch)) {
                return comp.to_string();
            }
            let x_major = is_x(&major);
            let x_minor = x_major || is_x(&minor);
            let any_x = x_minor || is_x(&patch);
            if gtlt == "=" && any_x {
                gtlt.clear();
            }
            if x_major {
                if gtlt == ">" || gtlt == "<" {
                    "<0.0.0-0".to_string()
                } else {
                    "*".to_string()
                }
            } else if !gtlt.is_empty() && any_x {
                if x_minor {
                    minor = "0".into();
                }
                patch = "0".into();
                if gtlt == ">" {
                    gtlt = ">=".into();
                    if x_minor {
                        major = inc(&major);
                        minor = "0".into();
                    } else {
                        minor = inc(&minor);
                    }
                    patch = "0".into();
                } else if gtlt == "<=" {
                    gtlt = "<".into();
                    if x_minor {
                        major = inc(&major);
                    } else {
                        minor = inc(&minor);
                    }
                }
                format!("{gtlt}{major}.{minor}.{patch}-0")
            } else if x_minor {
                format!(">={major}.0.0-0 <{}.0.0-0", inc(&major))
            } else if any_x {
                format!(">={major}.{minor}.0-0 <{major}.{}.0-0", inc(&minor))
            } else {
                matched
            }
        })
        .into_owned()
}

fn parse_comparator(comp: &str) -> String {
    let comp = G.build.replace_all(comp, "");
    let comp = words(&comp).into_iter().map(replace_caret).collect::<Vec<_>>().join(" ");
    let comp = words(&comp).into_iter().map(replace_tilde).collect::<Vec<_>>().join(" ");
    let comp = if comp.trim().is_empty() { String::new() } else { comp.split_whitespace().map(replace_xrange).collect::<Vec<_>>().join(" ") };
    G.star.replace(comp.trim(), "").into_owned()
}

fn hyphen_replace(c: &Captures) -> String {
    let (from, from_major, from_minor, from_patch, from_pre) = (group(c, 1), group(c, 2), group(c, 3), group(c, 4), group(c, 5));
    let (to_major, to_minor, to_patch, to_pre) = (group(c, 8), group(c, 9), group(c, 10), group(c, 11));
    let from = if is_x(from_major) {
        String::new()
    } else if is_x(from_minor) {
        format!(">={from_major}.0.0-0")
    } else if is_x(from_patch) {
        format!(">={from_major}.{from_minor}.0-0")
    } else if !from_pre.is_empty() {
        format!(">={from}")
    } else {
        format!(">={from}-0")
    };
    let to = if is_x(to_major) {
        String::new()
    } else if is_x(to_minor) {
        format!("<{}.0.0-0", inc(to_major))
    } else if is_x(to_patch) {
        format!("<{to_major}.{}.0-0", inc(to_minor))
    } else if !to_pre.is_empty() {
        format!("<={to_major}.{to_minor}.{to_patch}-{to_pre}")
    } else {
        format!("<{to_major}.{to_minor}.{}-0", inc(to_patch))
    };
    format!("{from} {to}").trim().to_string()
}

fn parse_range(range: &str) -> Option<Vec<Comparator>> {
    let range = G.build.replace_all(range, "").into_owned();
    let range = G.hyphen.replace(&range, hyphen_replace).into_owned();
    let range = G.comparator_trim.replace_all(&range, |c: &Captures| format!("{}{}{}", group(c, 1), group(c, 2), group(c, 3))).into_owned();
    let range = G.tilde_trim.replace_all(&range, |c: &Captures| format!("{}~", group(c, 1))).into_owned();
    let range = G.caret_trim.replace_all(&range, |c: &Captures| format!("{}^", group(c, 1))).into_owned();
    let joined = range.split(' ').map(parse_comparator).collect::<Vec<_>>().join(" ");
    let list: Vec<String> = joined
        .split_whitespace()
        .map(|w| if G.gte0pre.is_match(w) { String::new() } else { w.to_string() })
        .collect();
    let list = if list.is_empty() { vec![String::new()] } else { list };
    let mut out: Vec<Comparator> = Vec::new();
    for text in &list {
        let comparator = Comparator::new(text)?;
        if comparator.is_null_set() {
            return Some(vec![comparator]);
        }
        match out.iter().position(|c| c.value == comparator.value) {
            Some(i) => out[i] = comparator,
            None => out.push(comparator),
        }
    }
    if out.len() > 1 {
        out.retain(|c| !c.value.is_empty());
    }
    Some(out)
}

#[derive(Debug, Clone)]
pub struct Range {
    set: Vec<Vec<Comparator>>,
}

impl Range {
    pub fn parse(text: &str) -> Option<Range> {
        let raw = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut set: Vec<Vec<Comparator>> = Vec::new();
        for part in raw.split("||") {
            let comparators = parse_range(part.trim())?;
            if !comparators.is_empty() {
                set.push(comparators);
            }
        }
        if set.is_empty() {
            return None;
        }
        if set.len() > 1 {
            let first = set[0].clone();
            set.retain(|c| !c[0].is_null_set());
            if set.is_empty() {
                set = vec![first];
            } else if set.len() > 1 {
                if let Some(any) = set.iter().find(|c| c.len() == 1 && c[0].value.is_empty()).cloned() {
                    set = vec![any];
                }
            }
        }
        Some(Range { set })
    }

    fn format(&self) -> String {
        self.set
            .iter()
            .map(|comps| comps.iter().map(|c| c.value.trim().to_string()).collect::<Vec<_>>().join(" "))
            .collect::<Vec<_>>()
            .join("||")
    }

    fn test(&self, version: &SemVer) -> bool {
        self.set.iter().any(|set| set.iter().all(|c| c.test(version)))
    }
}

/// `semver.satisfies(version, range, { includePrerelease: true })`.
pub fn satisfies(version: &str, range: &str) -> bool {
    match (SemVer::parse(version), Range::parse(range)) {
        (Some(v), Some(r)) => r.test(&v),
        _ => false,
    }
}

/// `semver.validRange(range, { includePrerelease: true })`.
pub fn valid_range(range: &str) -> Option<String> {
    let formatted = Range::parse(range)?.format();
    Some(if formatted.is_empty() { "*".into() } else { formatted })
}

/// `semver.subset(sub, dom, { includePrerelease: true })`; false when either side is not a range.
pub fn subset(sub: &str, dom: &str) -> bool {
    if sub == dom {
        return true;
    }
    let (Some(sub), Some(dom)) = (Range::parse(sub), Range::parse(dom)) else { return false };
    let mut saw_non_null = false;
    'outer: for simple_sub in &sub.set {
        for simple_dom in &dom.set {
            let verdict = simple_subset(simple_sub, simple_dom);
            saw_non_null = saw_non_null || verdict.is_some();
            if verdict == Some(true) {
                continue 'outer;
            }
        }
        if saw_non_null {
            return false;
        }
    }
    true
}

fn is_any(set: &[Comparator]) -> bool {
    set.len() == 1 && set[0].semver.is_none()
}

fn sem(c: &Comparator) -> &SemVer {
    c.semver.as_ref().expect("non-ANY comparator")
}

fn satisfies_comparator(version: &SemVer, c: &Comparator) -> bool {
    Range::parse(&c.value).is_some_and(|r| r.test(version))
}

fn higher_gt<'a>(a: Option<&'a Comparator>, b: &'a Comparator) -> &'a Comparator {
    let Some(a) = a else { return b };
    match sem(a).compare(sem(b)) {
        Ordering::Greater => a,
        Ordering::Less => b,
        Ordering::Equal if b.operator == ">" && a.operator == ">=" => b,
        Ordering::Equal => a,
    }
}

fn lower_lt<'a>(a: Option<&'a Comparator>, b: &'a Comparator) -> &'a Comparator {
    let Some(a) = a else { return b };
    match sem(a).compare(sem(b)) {
        Ordering::Less => a,
        Ordering::Greater => b,
        Ordering::Equal if b.operator == "<" && a.operator == "<=" => b,
        Ordering::Equal => a,
    }
}

/// `Some(true/false)`, or `None` for the null set.
fn simple_subset(sub: &[Comparator], dom: &[Comparator]) -> Option<bool> {
    let min_pre = [Comparator::new(">=0.0.0-0")?];
    let sub: &[Comparator] = if is_any(sub) {
        if is_any(dom) {
            return Some(true);
        }
        &min_pre
    } else {
        sub
    };
    if is_any(dom) {
        return Some(true);
    }
    let mut eq_set: Vec<&SemVer> = Vec::new();
    let (mut gt, mut lt): (Option<&Comparator>, Option<&Comparator>) = (None, None);
    for c in sub {
        match c.operator.as_str() {
            ">" | ">=" => gt = Some(higher_gt(gt, c)),
            "<" | "<=" => lt = Some(lower_lt(lt, c)),
            _ => eq_set.push(sem(c)),
        }
    }
    if eq_set.len() > 1 {
        return None;
    }
    let mut gtlt_comp: Option<Ordering> = None;
    if let (Some(g), Some(l)) = (gt, lt) {
        let ord = sem(g).compare(sem(l));
        gtlt_comp = Some(ord);
        if ord == Ordering::Greater || (ord == Ordering::Equal && (g.operator != ">=" || l.operator != "<=")) {
            return None;
        }
    }
    if let Some(eq) = eq_set.first() {
        if gt.is_some_and(|g| !satisfies_comparator(eq, g)) || lt.is_some_and(|l| !satisfies_comparator(eq, l)) {
            return None;
        }
        return Some(dom.iter().all(|c| satisfies_comparator(eq, c)));
    }
    let (mut has_dom_lt, mut has_dom_gt) = (false, false);
    // includePrerelease mode: no comparator needs a same-tuple prerelease partner (needDom*Pre is false).
    for c in dom {
        has_dom_gt = has_dom_gt || c.operator == ">" || c.operator == ">=";
        has_dom_lt = has_dom_lt || c.operator == "<" || c.operator == "<=";
        if let Some(g) = gt {
            if c.operator == ">" || c.operator == ">=" {
                let higher = higher_gt(Some(g), c);
                if std::ptr::eq(higher, c) && !std::ptr::eq(higher, g) {
                    return Some(false);
                }
            } else if g.operator == ">=" && !c.test(sem(g)) {
                return Some(false);
            }
        }
        if let Some(l) = lt {
            if c.operator == "<" || c.operator == "<=" {
                let lower = lower_lt(Some(l), c);
                if std::ptr::eq(lower, c) && !std::ptr::eq(lower, l) {
                    return Some(false);
                }
            } else if l.operator == "<=" && !c.test(sem(l)) {
                return Some(false);
            }
        }
        if c.operator.is_empty() && (lt.is_some() || gt.is_some()) && gtlt_comp != Some(Ordering::Equal) {
            return Some(false);
        }
    }
    if gt.is_some() && has_dom_lt && lt.is_none() && gtlt_comp != Some(Ordering::Equal) {
        return Some(false);
    }
    if lt.is_some() && has_dom_gt && gt.is_none() && gtlt_comp != Some(Ordering::Equal) {
        return Some(false);
    }
    Some(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grammar() {
        assert_eq!(valid_range("^4.0.5").as_deref(), Some(">=4.0.5 <5.0.0-0"));
        assert_eq!(valid_range(">=4.0.5 <5.0.0").as_deref(), Some(">=4.0.5 <5.0.0"));
        assert_eq!(valid_range("4.x").as_deref(), Some(">=4.0.0-0 <5.0.0-0"));
        assert_eq!(valid_range("*").as_deref(), Some("*"));
        assert_eq!(valid_range("workspace:*"), None);
        assert!(subset(&valid_range("^4.0.5").unwrap(), ">=4.0.5 <5.0.0"));
        assert!(!subset(&valid_range("^4.0.0").unwrap(), ">=4.0.5 <5.0.0"));
        assert!(!subset(&valid_range("^3 || ^4").unwrap(), ">=4.0.5 <5.0.0"));
        assert!(satisfies("4.0.5", "^4.0.0"));
        assert!(satisfies("4.1.0-beta.1", ">=4 <5"));
        assert!(!satisfies("3.9.0", "^4"));
    }
}
