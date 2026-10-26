//! Plain URL detection. Byte ranges are converted to terminal columns by
//! the caller, so Unicode and wide cells never use byte offsets as columns.

use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub columns: Range<u16>,
    pub uri: String,
}

pub fn urls(text: &str) -> Vec<Range<usize>> {
    let mut found = Vec::new();
    let mut offset = 0;
    while offset < text.len() {
        let remaining = &text[offset..];
        let next = [remaining.find("https://"), remaining.find("http://")]
            .into_iter()
            .flatten()
            .min();
        let Some(next) = next else { break };
        let start = offset + next;
        let prefix = if text[start..].starts_with("https://") {
            8
        } else {
            7
        };
        let mut end = start
            + text[start..]
                .find(|c: char| {
                    c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | '\'' | '"' | '`')
                })
                .unwrap_or(text.len() - start);
        while end > start + prefix {
            let Some(last) = text[..end].chars().next_back() else {
                break;
            };
            let unbalanced = match last {
                ')' | ']' | '}' => {
                    let open = match last {
                        ')' => '(',
                        ']' => '[',
                        _ => '{',
                    };
                    let url = &text[start..end];
                    url.matches(last).count() > url.matches(open).count()
                }
                _ => false,
            };
            if unbalanced || matches!(last, '.' | ',' | ';' | ':' | '!' | '?') {
                end -= last.len_utf8();
            } else {
                break;
            }
        }
        let boundary = text[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric() && !matches!(c, '_' | '/' | ':' | '.'));
        let authority = &text[start + prefix..end];
        if boundary && !authority.is_empty() && !authority.starts_with(['/', '?', '#']) {
            found.push(start..end);
        }
        offset = (start + prefix).max(end);
    }
    found
}

/// OSC 8 targets must be URI data, never an xdg-open option or shell input.
pub fn is_uri(uri: &str) -> bool {
    let Some((scheme, _)) = uri.split_once(':') else {
        return false;
    };
    scheme
        .as_bytes()
        .first()
        .is_some_and(u8::is_ascii_alphabetic)
        && scheme
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"+.-".contains(&c))
        && !uri.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_boundaries_keep_balanced_paths_and_trim_prose_punctuation() {
        let text =
            "界 (https://example.org/a_(b)), <http://[::1]:8080/q?a=b&c=d>. https://x.test/end!";
        assert_eq!(
            urls(text).into_iter().map(|r| &text[r]).collect::<Vec<_>>(),
            [
                "https://example.org/a_(b)",
                "http://[::1]:8080/q?a=b&c=d",
                "https://x.test/end",
            ]
        );
        let text = "www.example.org ftp://example.org http:// https:///bad xhttps://bad.test https://ok.test/é#part";
        assert_eq!(
            urls(text).into_iter().map(|r| &text[r]).collect::<Vec<_>>(),
            ["https://ok.test/é#part"]
        );
    }

    #[test]
    fn opener_targets_are_uris_and_cannot_be_command_options() {
        assert!(is_uri("file:///tmp/a b"));
        assert!(is_uri("mailto:someone@example.org"));
        for invalid in ["--help", "relative/path", "https://x\n--help", ":empty"] {
            assert!(!is_uri(invalid));
        }
    }
}
