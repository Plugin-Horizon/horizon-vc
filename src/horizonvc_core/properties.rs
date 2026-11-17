//! Reader for Java `.properties` files, following `java.util.Properties.load` for what the original
//! Simple Voice Chat files use: `#` and `!` comment lines, `key=value`, `key:value` or `key value`,
//! trailing-backslash line continuation, and the escapes `\t \n \r \f \uXXXX` (any other `\c` is `c`).
//! Values keep their trailing whitespace, as Java does.

/// Every key/value pair of a file, in file order (a repeated key appears twice; the last one wins for a
/// reader that keeps a map).
pub fn parse(text: &str) -> Vec<(String, String)> {
    logical_lines(text)
        .iter()
        .filter_map(|line| split_pair(line))
        .collect()
}

/// Joins continuation lines and drops comments and blank lines.
fn logical_lines(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    let mut continuing = false;
    for raw in text.lines() {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        let line = if continuing {
            raw.trim_start()
        } else {
            let t = raw.trim_start();
            if t.is_empty() || t.starts_with('#') || t.starts_with('!') {
                continue;
            }
            t
        };
        // An odd number of trailing backslashes continues the line; the backslash itself is dropped.
        let trailing = line.len() - line.trim_end_matches('\\').len();
        if trailing % 2 == 1 {
            cur.push_str(&line[..line.len() - 1]);
            continuing = true;
        } else {
            cur.push_str(line);
            lines.push(std::mem::take(&mut cur));
            continuing = false;
        }
    }
    if continuing {
        lines.push(cur);
    }
    lines
}

fn split_pair(line: &str) -> Option<(String, String)> {
    let mut escaped = false;
    let mut end = line.len();
    for (i, c) in line.char_indices() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '=' || c == ':' || c.is_whitespace() {
            end = i;
            break;
        }
    }
    let key = unescape(&line[..end]);
    if key.is_empty() {
        return None;
    }
    let rest = line[end..].trim_start();
    let rest = rest
        .strip_prefix(|c| c == '=' || c == ':')
        .map(str::trim_start)
        .unwrap_or(rest);
    Some((key, unescape(rest)))
}

/// Resolves the escapes of a key or value.
pub fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            None => {}
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('f') => out.push('\x0c'),
            Some('u') => {
                let hex: String = chars.by_ref().take(4).collect();
                match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    Some(ch) if hex.len() == 4 => out.push(ch),
                    _ => {
                        out.push_str("\\u");
                        out.push_str(&hex);
                    }
                }
            }
            Some(other) => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get(text: &str, key: &str) -> Option<String> {
        parse(text)
            .into_iter()
            .rev()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }

    #[test]
    fn separators_and_comments() {
        let t = "# a comment\n! another\n  a=1\nb : 2\nc\t3\nd\n\n";
        assert_eq!(get(t, "a").as_deref(), Some("1"));
        assert_eq!(get(t, "b").as_deref(), Some("2"));
        assert_eq!(get(t, "c").as_deref(), Some("3"));
        assert_eq!(get(t, "d").as_deref(), Some(""));
        assert_eq!(parse(t).len(), 4);
    }

    #[test]
    fn values_keep_inner_and_trailing_spaces_and_drop_leading_ones() {
        assert_eq!(get("k =  a b  \n", "k").as_deref(), Some("a b  "));
        assert_eq!(get("k=a=b:c\n", "k").as_deref(), Some("a=b:c"));
    }

    #[test]
    fn escapes_are_resolved() {
        assert_eq!(get("k=a\\tb\\nc\\\\d\\=e\n", "k").as_deref(), Some("a\tb\nc\\d=e"));
        assert_eq!(get("k=\\u0041\\u00e4\n", "k").as_deref(), Some("A\u{e4}"));
        assert_eq!(get("k=\\x\n", "k").as_deref(), Some("x"));
        assert_eq!(get("k=\\\\n\n", "k").as_deref(), Some("\\n"));
    }

    #[test]
    fn continuation_lines_join_and_strip_their_indent() {
        let t = "k=one \\\n    two\\\n three\nnext=1\n";
        assert_eq!(get(t, "k").as_deref(), Some("one twothree"));
        assert_eq!(get(t, "next").as_deref(), Some("1"));
    }

    #[test]
    fn a_doubled_backslash_at_the_end_is_not_a_continuation() {
        assert_eq!(get("k=end\\\\\nnext=1\n", "next").as_deref(), Some("1"));
    }

    #[test]
    fn crlf_line_endings_are_handled() {
        assert_eq!(get("a=1\r\nb=2\r\n", "b").as_deref(), Some("2"));
    }
}
