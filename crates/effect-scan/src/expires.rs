//! `eff-ignore ... expires="..."` as an instant: the oracle compared `Date.parse(expires)` with
//! `Date.now()`, so the contract is "parse to epoch milliseconds, compare numerically". Anything
//! that does not parse is a finding, never a silent pass.
//!
//! Supported (documented in references/scanner-manifest.md):
//! - ISO 8601: `YYYY`, `YYYY-MM`, `YYYY-MM-DD`, `YYYY-MM-DDTHH:MM[:SS[.fff]][Z|+HH:MM]`
//!   (`T` or a space; `±YYYYYY` extended years).
//! - Common JS forms: `Jan 1 2020`, `January 1, 2020`, `Wed Jan 1 2020`, `1 Jan 2020`,
//!   `12/31/2099`, each with an optional `HH:MM[:SS]` and `GMT|UTC|Z|±HHMM|±HH:MM`.
//!
//! Zone-less date-times and the non-ISO forms are read as UTC (JS reads them in the local zone);
//! the difference is at most the UTC offset on the expiry day.
use regex::Regex;
use std::sync::LazyLock;

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

struct Parts {
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
    millis: i64,
    offset_minutes: i64,
}

fn to_ms(p: Parts) -> Option<i64> {
    let valid_time = (p.hour < 24 && p.minute < 60 && p.second < 60) || (p.hour == 24 && p.minute == 0 && p.second == 0 && p.millis == 0);
    if !(1..=12).contains(&p.month) || p.day < 1 || p.day > days_in_month(p.year, p.month) || !valid_time {
        return None;
    }
    let secs = days_from_civil(p.year, p.month, p.day) * 86_400 + p.hour * 3600 + p.minute * 60 + p.second - p.offset_minutes * 60;
    Some(secs * 1000 + p.millis)
}

fn offset(text: &str) -> Option<i64> {
    let digits: String = text[1..].chars().filter(|c| *c != ':').collect();
    let (h, m): (i64, i64) = match digits.len() {
        2 => (digits.parse().ok()?, 0),
        4 => (digits[..2].parse().ok()?, digits[2..].parse().ok()?),
        _ => return None,
    };
    let minutes = h * 60 + m;
    Some(if text.starts_with('-') { -minutes } else { minutes })
}

fn num(s: Option<regex::Match>) -> i64 {
    s.and_then(|m| m.as_str().parse().ok()).unwrap_or(0)
}

fn month_number(name: &str) -> Option<i64> {
    const MONTHS: [&str; 12] = ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"];
    let lower = name.to_ascii_lowercase();
    MONTHS.iter().position(|m| lower.len() >= 3 && m.starts_with(&lower)).map(|i| i as i64 + 1)
}

fn parse_iso(s: &str) -> Option<i64> {
    static ISO: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^([+-]\d{6}|\d{4})(?:-(\d{2})(?:-(\d{2}))?)?(?:[T ](\d{2}):(\d{2})(?::(\d{2})(?:\.(\d+))?)?\s*(Z|[+-]\d{2}:?\d{2})?)?$").unwrap()
    });
    let c = ISO.captures(s)?;
    let year_text = c.get(1)?.as_str();
    if year_text == "-000000" {
        return None;
    }
    let millis = c.get(7).map(|m| format!("{:0<3}", &m.as_str()[..m.as_str().len().min(3)]).parse().unwrap_or(0)).unwrap_or(0);
    to_ms(Parts {
        year: year_text.parse().ok()?,
        month: c.get(2).map_or(1, |m| m.as_str().parse().unwrap_or(0)),
        day: c.get(3).map_or(1, |m| m.as_str().parse().unwrap_or(0)),
        hour: num(c.get(4)),
        minute: num(c.get(5)),
        second: num(c.get(6)),
        millis,
        offset_minutes: match c.get(8).map(|m| m.as_str()) {
            None | Some("Z") => 0,
            Some(z) => offset(z)?,
        },
    })
}

fn parse_legacy(s: &str) -> Option<i64> {
    const TIME: &str = r"(?:\s+(\d{1,2}):(\d{2})(?::(\d{2}))?)?(?:\s*(?:GMT|UTC|Z)?\s*([+-]\d{2}:?\d{2})?)?";
    static US: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"^(\d{{1,2}})/(\d{{1,2}})/(\d{{4}}){TIME}$")).unwrap());
    static MDY: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"^(?:[A-Za-z]{{3,9}},?\s+)?([A-Za-z]{{3,9}})\.?\s+(\d{{1,2}}),?\s+(\d{{4}}){TIME}$")).unwrap());
    static DMY: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"^(?:[A-Za-z]{{3,9}},?\s+)?(\d{{1,2}})\s+([A-Za-z]{{3,9}})\.?,?\s+(\d{{4}}){TIME}$")).unwrap());
    let (year, month, day, rest) = if let Some(c) = US.captures(s) {
        (num(c.get(3)), num(c.get(1)), num(c.get(2)), c)
    } else if let Some(c) = MDY.captures(s) {
        (num(c.get(3)), month_number(c.get(1)?.as_str())?, num(c.get(2)), c)
    } else if let Some(c) = DMY.captures(s) {
        (num(c.get(3)), month_number(c.get(2)?.as_str())?, num(c.get(1)), c)
    } else {
        return None;
    };
    to_ms(Parts {
        year,
        month,
        day,
        hour: num(rest.get(4)),
        minute: num(rest.get(5)),
        second: num(rest.get(6)),
        millis: 0,
        offset_minutes: match rest.get(7) {
            Some(z) => offset(z.as_str())?,
            None => 0,
        },
    })
}

/// Epoch milliseconds of `expires`, or `None` when it is not a date this scanner understands.
pub fn parse_expires(text: &str) -> Option<i64> {
    let s = text.trim();
    parse_iso(s).or_else(|| parse_legacy(s))
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_and_js_forms_agree_on_the_instant() {
        let ms = parse_expires("2020-01-01").unwrap();
        assert_eq!(ms, 1_577_836_800_000);
        assert_eq!(parse_expires("2020-01-01T00:00:00Z"), Some(ms));
        assert_eq!(parse_expires("2020-01-01T00:00:00.000Z"), Some(ms));
        assert_eq!(parse_expires("2020-01-01T02:00:00+02:00"), Some(ms));
        assert_eq!(parse_expires("Jan 1 2020"), Some(ms));
        assert_eq!(parse_expires("January 1, 2020"), Some(ms));
        assert_eq!(parse_expires("1/1/2020"), Some(ms));
        assert_eq!(parse_expires("1 Jan 2020"), Some(ms));
        assert!(parse_expires("12/31/2099").unwrap() > now_ms());
    }

    #[test]
    fn invalid_dates_do_not_parse() {
        for bad in ["", "soon", "2020-13-01", "2021-02-29", "2020-01-01T25:00:00Z", "31/12/2099", "Foo 1 2020"] {
            assert_eq!(parse_expires(bad), None, "{bad}");
        }
    }
}
