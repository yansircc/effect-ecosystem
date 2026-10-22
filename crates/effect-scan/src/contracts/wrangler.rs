//! Minimal Wrangler config reader: `wrangler.json` / `wrangler.jsonc` as a JSON value.
//!
//! Deliberately small and free of scan state so the runtime-facts / package stages can reuse it:
//! `parse(path, text)` yields the top-level object (comments and trailing commas tolerated, like
//! the JSONC parser the TypeScript scanner uses); `WranglerConfig` exposes the fields execution
//! contracts read. Line numbers for runtime facts are not provided here.
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum WranglerError {
    /// Only `.json` / `.jsonc` are readable.
    UnsupportedFormat,
    /// Not JSONC, or not a top-level object.
    Invalid,
}

impl WranglerError {
    /// Stable code shared with the TypeScript scanner's `parseWranglerConfig`.
    pub fn code(&self) -> &'static str {
        match self {
            WranglerError::UnsupportedFormat => "unsupported-wrangler-format",
            WranglerError::Invalid => "invalid-wrangler-config",
        }
    }
}

#[derive(Debug, Clone)]
pub struct WranglerConfig {
    pub value: Value,
}

impl WranglerConfig {
    /// `main`, only when it is a string.
    pub fn main(&self) -> Option<&str> {
        self.value.get("main").and_then(Value::as_str)
    }

    pub fn durable_object_bindings(&self) -> &[Value] {
        self.value.get("durable_objects").and_then(|d| d.get("bindings")).and_then(Value::as_array).map_or(&[], Vec::as_slice)
    }

    pub fn workflows(&self) -> &[Value] {
        self.value.get("workflows").and_then(Value::as_array).map_or(&[], Vec::as_slice)
    }
}

pub fn parse(path: &str, text: &str) -> Result<WranglerConfig, WranglerError> {
    if !path.ends_with(".json") && !path.ends_with(".jsonc") {
        return Err(WranglerError::UnsupportedFormat);
    }
    match serde_json::from_str::<Value>(&strip_jsonc(text)) {
        Ok(value @ Value::Object(_)) => Ok(WranglerConfig { value }),
        _ => Err(WranglerError::Invalid),
    }
}

/// Removes `//` and `/* */` comments and trailing commas outside of strings.
fn strip_jsonc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            match c {
                '\\' => out.extend(chars.next()),
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '/' if chars.peek() == Some(&'/') => {
                for next in chars.by_ref() {
                    if next == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut previous = '\0';
                for next in chars.by_ref() {
                    if previous == '*' && next == '/' {
                        break;
                    }
                    if next == '\n' {
                        out.push('\n');
                    }
                    previous = next;
                }
                out.push(' ');
            }
            '}' | ']' => {
                let trimmed = out.trim_end().len();
                if out[..trimmed].ends_with(',') {
                    out.truncate(trimmed - 1);
                }
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_jsonc() {
        let config = parse("wrangler.jsonc", "{\n // main\n \"main\": \"./src/a.ts\", /* x */\n \"workflows\": [{\"class_name\": \"W\",},],\n}").unwrap();
        assert_eq!(config.main(), Some("./src/a.ts"));
        assert_eq!(config.workflows().len(), 1);
    }

    #[test]
    fn comment_markers_inside_strings_survive() {
        let config = parse("wrangler.json", r#"{"main": "http://x//y,]"}"#).unwrap();
        assert_eq!(config.main(), Some("http://x//y,]"));
    }

    #[test]
    fn rejects_other_formats_and_non_objects() {
        assert_eq!(parse("wrangler.toml", "").unwrap_err(), WranglerError::UnsupportedFormat);
        assert_eq!(parse("wrangler.json", "[1]").unwrap_err(), WranglerError::Invalid);
    }
}
