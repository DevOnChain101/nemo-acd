//! Accessibility tree (Accessibility.getFullAXTree) -> compact text with stable refs.

use serde_json::Value;
use std::collections::HashMap;

/// Maps DOM backend node ids to short refs (`e12`). A node keeps its ref across snapshots.
#[derive(Default)]
pub struct RefTable {
    by_backend: HashMap<i64, u32>,
    by_ref: HashMap<u32, i64>,
    next: u32,
}

impl RefTable {
    pub fn alloc(&mut self, backend: i64) -> u32 {
        if let Some(r) = self.by_backend.get(&backend) {
            return *r;
        }
        self.next += 1;
        self.by_backend.insert(backend, self.next);
        self.by_ref.insert(self.next, backend);
        self.next
    }

    pub fn resolve(&self, r: &str) -> Option<i64> {
        let n: u32 = r.strip_prefix('e')?.parse().ok()?;
        self.by_ref.get(&n).copied()
    }
}

pub struct Rendered {
    pub text: String,
    pub truncated: bool,
}

const INTERACTIVE: &[&str] = &[
    "button",
    "link",
    "textbox",
    "searchbox",
    "checkbox",
    "radio",
    "combobox",
    "menuitem",
    "menuitemcheckbox",
    "menuitemradio",
    "tab",
    "switch",
    "slider",
    "spinbutton",
    "option",
    "listbox",
    "treeitem",
    "PopUpButton",
    "DisclosureTriangle",
];
const TRANSPARENT: &[&str] = &[
    "",
    "none",
    "presentation",
    "generic",
    "InlineTextBox",
    "LineBreak",
];
const FLAGS: &[&str] = &[
    "disabled", "expanded", "selected", "required", "readonly", "focused",
];
const MAX_DEPTH: usize = 100;

fn clip(s: &str) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() > 200 {
        format!("{}…", s.chars().take(200).collect::<String>())
    } else {
        s
    }
}

fn quoted(s: &str) -> String {
    serde_json::to_string(&clip(s)).unwrap_or_default()
}

fn truthy(v: &Value) -> bool {
    v.as_bool() == Some(true) || v.as_str() == Some("true")
}

pub fn render(nodes: &[Value], refs: &mut RefTable, max_nodes: usize) -> Rendered {
    let by_id: HashMap<&str, &Value> = nodes
        .iter()
        .filter_map(|n| Some((n.get("nodeId")?.as_str()?, n)))
        .collect();
    let mut out = String::new();
    let mut lines = 0;
    let mut truncated = false;
    let mut stack: Vec<(&Value, usize)> = nodes.first().map(|n| (n, 0)).into_iter().collect();
    while let Some((n, depth)) = stack.pop() {
        let ignored = n.get("ignored").and_then(Value::as_bool) == Some(true);
        let role = n["role"]["value"].as_str().unwrap_or("");
        let name = n["name"]["value"].as_str().unwrap_or("");
        let mut child_depth = depth;
        let mut descend = depth < MAX_DEPTH;
        if !ignored && !TRANSPARENT.contains(&role) {
            if role == "StaticText" {
                descend = false;
                if name.trim().is_empty() {
                    continue;
                }
            }
            if lines >= max_nodes {
                truncated = true;
                break;
            }
            lines += 1;
            child_depth = depth + 1;
            out.push_str(&"  ".repeat(depth));
            let label = if role == "StaticText" { "text" } else { role };
            out.push_str(&format!("- {label}"));
            if !name.trim().is_empty() {
                out.push_str(&format!(" {}", quoted(name)));
            }
            if let (true, Some(b)) = (
                INTERACTIVE.contains(&role),
                n.get("backendDOMNodeId").and_then(Value::as_i64),
            ) {
                out.push_str(&format!(" [ref=e{}]", refs.alloc(b)));
            }
            for p in n["properties"].as_array().into_iter().flatten() {
                let (pn, pv) = (p["name"].as_str().unwrap_or(""), &p["value"]["value"]);
                if pn == "checked" && pv.as_str() == Some("mixed") {
                    out.push_str(" [checked=mixed]");
                } else if (pn == "checked" || FLAGS.contains(&pn)) && truthy(pv) {
                    out.push_str(&format!(" [{pn}]"));
                } else if pn == "level" {
                    out.push_str(&format!(" [level={pv}]"));
                }
            }
            if let Some(v) = n["value"]["value"].as_str().filter(|v| !v.is_empty()) {
                out.push_str(&format!(" value={}", quoted(v)));
            }
            out.push('\n');
        }
        if descend {
            let kids = n["childIds"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str);
            for k in kids.rev() {
                if let Some(c) = by_id.get(k) {
                    stack.push((c, child_depth));
                }
            }
        }
    }
    Rendered {
        text: out,
        truncated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> Vec<Value> {
        vec![
            json!({"nodeId":"1","role":{"value":"RootWebArea"},"name":{"value":"Demo"},"childIds":["2","3","6"]}),
            json!({"nodeId":"2","role":{"value":"heading"},"name":{"value":"Sign in"},"properties":[{"name":"level","value":{"value":1}}],"childIds":["7"]}),
            json!({"nodeId":"3","role":{"value":"generic"},"childIds":["4","5"]}),
            json!({"nodeId":"4","role":{"value":"textbox"},"name":{"value":"Email \"work\""},"value":{"value":"a@b.c"},"backendDOMNodeId":40,"properties":[{"name":"required","value":{"value":true}}]}),
            json!({"nodeId":"5","role":{"value":"checkbox"},"name":{"value":"Remember"},"backendDOMNodeId":50,"properties":[{"name":"checked","value":{"value":"mixed"}}]}),
            json!({"nodeId":"6","role":{"value":"button"},"name":{"value":"Go"},"backendDOMNodeId":60,"ignored":false}),
            json!({"nodeId":"7","role":{"value":"StaticText"},"name":{"value":"Sign in"},"childIds":["8"]}),
            json!({"nodeId":"8","role":{"value":"InlineTextBox"},"name":{"value":"Sign in"}}),
        ]
    }

    #[test]
    fn renders_compact_tree_with_refs() {
        let mut refs = RefTable::default();
        let r = render(&fixture(), &mut refs, 100);
        let want = "\
- RootWebArea \"Demo\"
  - heading \"Sign in\" [level=1]
    - text \"Sign in\"
  - textbox \"Email \\\"work\\\"\" [ref=e1] [required] value=\"a@b.c\"
  - checkbox \"Remember\" [ref=e2] [checked=mixed]
  - button \"Go\" [ref=e3]
";
        assert_eq!(r.text, want);
        assert!(!r.truncated);
        assert_eq!(refs.resolve("e3"), Some(60));
        assert_eq!(refs.resolve("e9"), None);
        assert_eq!(refs.resolve("x1"), None);
    }

    #[test]
    fn refs_are_stable_across_snapshots_and_truncation_is_reported() {
        let mut refs = RefTable::default();
        render(&fixture(), &mut refs, 100);
        let mut shuffled = fixture();
        shuffled.swap(3, 5); // order in the array must not matter
        render(&shuffled, &mut refs, 100);
        assert_eq!(refs.alloc(60), 3);
        assert_eq!(refs.alloc(99), 4);
        let r = render(&fixture(), &mut refs, 2);
        assert!(r.truncated);
        assert_eq!(r.text.lines().count(), 2);
    }
}
