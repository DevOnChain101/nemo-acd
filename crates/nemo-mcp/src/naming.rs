//! Provider-safe tool names `mcp__<server>__<tool>` (port of mcp.js:115-154).

use sha1::{Digest, Sha1};

pub const MAX_TOOL_NAME_LEN: usize = 64;

/// Replaces every UTF-16 code unit outside `[A-Za-z0-9_-]` with `_` (JS `replace` without the `u`
/// flag works per code unit, so an astral char becomes two underscores). Empty becomes `_`.
fn sanitize_part(s: &str) -> String {
    let clean: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c.to_string()
            } else {
                "_".repeat(c.len_utf16())
            }
        })
        .collect();
    if clean.is_empty() { "_".into() } else { clean }
}

/// Wire name for a server tool. Over-long names are truncated and get a 6-hex sha1 suffix.
/// Distinct inputs can still sanitize to the same name (as in Node); the later one wins in the
/// manager's reverse table.
pub fn tool_name_for(server: &str, tool: &str) -> String {
    let s = sanitize_part(server);
    let t = sanitize_part(tool);
    let full = format!("mcp__{s}__{t}");
    if full.len() <= MAX_TOOL_NAME_LEN {
        return full;
    }
    let hash: String = Sha1::digest(format!("{server}\0{tool}").as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()[..6]
        .to_string();
    let budget = MAX_TOOL_NAME_LEN - 5 - 2 - 2 - hash.len();
    let half = (budget / 2).max(1);
    let s_trunc = &s[..half.min(s.len())];
    let t_trunc = &t[..(budget - s_trunc.len()).min(t.len())];
    format!("mcp__{s_trunc}__{t_trunc}__{hash}")
}

/// Best-effort reverse for names we did not mint: split on the first `__` after `mcp__`.
pub fn parse_tool_name_fallback(name: &str) -> Option<(String, String)> {
    let (server, tool) = name.strip_prefix("mcp__")?.split_once("__")?;
    Some((server.into(), tool.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_names() {
        assert_eq!(tool_name_for("echo", "add"), "mcp__echo__add");
        assert_eq!(tool_name_for("my-srv_1", "t-2"), "mcp__my-srv_1__t-2");
    }

    #[test]
    fn sanitizes_and_empty_parts() {
        assert_eq!(
            tool_name_for("weird server!!", "tool/with:chars"),
            "mcp__weird_server____tool_with_chars"
        );
        assert_eq!(tool_name_for("", ""), "mcp______");
        // BMP char -> 1 underscore, astral char -> 2 (UTF-16 units)
        assert_eq!(tool_name_for("é", "😀"), "mcp_______");
    }

    #[test]
    fn exactly_64_is_untouched_65_is_hashed() {
        let s = "a".repeat(28);
        let t = "b".repeat(64 - 5 - 2 - 28);
        let n = tool_name_for(&s, &t);
        assert_eq!(n.len(), 64);
        assert_eq!(n, format!("mcp__{s}__{t}"));
        let n = tool_name_for(&s, &format!("{t}b"));
        assert_eq!(n.len(), 64);
        assert!(n.starts_with(&format!("mcp__{}__", &s[..24])));
    }

    #[test]
    fn long_names_truncate_with_stable_hash() {
        let n = tool_name_for(&"a".repeat(80), &"b".repeat(80));
        assert_eq!(n.len(), 64);
        let expected_hash = {
            let d = Sha1::digest(format!("{}\0{}", "a".repeat(80), "b".repeat(80)).as_bytes());
            format!("{:02x}{:02x}{:02x}", d[0], d[1], d[2])
        };
        assert_eq!(
            n,
            format!(
                "mcp__{}__{}__{}",
                "a".repeat(24),
                "b".repeat(25),
                expected_hash
            )
        );
        // same input, same name; different tool, different hash
        assert_eq!(n, tool_name_for(&"a".repeat(80), &"b".repeat(80)));
        assert_ne!(
            n,
            tool_name_for(&"a".repeat(80), &format!("{}c", "b".repeat(79)))
        );
        assert!(
            n.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        );
    }

    #[test]
    fn short_server_long_tool_gets_remaining_budget() {
        let n = tool_name_for("s", &"t".repeat(100));
        assert_eq!(n.len(), 5 + 1 + 2 + 48 + 2 + 6);
    }

    #[test]
    fn fallback_parse() {
        let p = |n| parse_tool_name_fallback(n);
        assert_eq!(p("mcp__echo__add"), Some(("echo".into(), "add".into())));
        assert_eq!(p("mcp__a_b__c__d"), Some(("a_b".into(), "c__d".into())));
        assert_eq!(p("mcp____x"), Some(("".into(), "x".into())));
        assert_eq!(p("mcp__a__"), Some(("a".into(), "".into())));
        assert_eq!(p("mcp__nosep"), None);
        assert_eq!(p("mcp___x"), None);
        assert_eq!(p("other__a__b"), None);
    }
}
