//! The honest status line under the frame: sandbox mode and what it really guarantees, budgets
//! against their limits, and the verification verdict with its evidence. Nothing here is
//! decoration: a missing fact is shown as missing ("unknown", "unverified"), never as OK.

use nemo_events::Budget;
use nemo_tui_render::theme::{GREEN, RED, T_INACTIVE, YELLOW, fit, paint};
use nemo_tui_render::util::{fmt_n, short, to_fixed};

use crate::chat::{Totals, Verification};

/// What the host built for command confinement. Strings, so this crate does not depend on the
/// sandbox backends: the CLI fills it from `SandboxMode` and `SandboxBackend::capabilities()`.
#[derive(Clone, Debug, Default)]
pub struct SandboxStatus {
    /// `enforce`, `best-effort`, `off`; empty = the host did not say.
    pub mode: String,
    pub backend: String,
    /// Guarantee levels as words (`strong`, `weak`, `none`).
    pub filesystem: String,
    pub network: String,
    /// Commands run (or may run) with the agent's full privileges because the user asked.
    pub unsandboxed_by_request: bool,
}

impl SandboxStatus {
    fn text(&self) -> (String, bool) {
        if self.mode.is_empty() {
            return ("sandbox: unknown".into(), true);
        }
        let mut s = format!("sandbox: {}", self.mode);
        let caps: Vec<String> = [
            (!self.backend.is_empty()).then(|| self.backend.clone()),
            (!self.filesystem.is_empty()).then(|| format!("fs:{}", self.filesystem)),
            (!self.network.is_empty()).then(|| format!("net:{}", self.network)),
        ]
        .into_iter()
        .flatten()
        .collect();
        if !caps.is_empty() {
            s.push_str(&format!(" [{}]", caps.join(" ")));
        }
        if self.unsandboxed_by_request {
            s.push_str(" UNSANDBOXED_BY_REQUEST");
        }
        let bad = self.unsandboxed_by_request || self.mode == "off";
        (s, bad)
    }
}

pub fn status_line(
    sb: &SandboxStatus,
    budget: &Budget,
    t: &Totals,
    verification: Option<&Verification>,
    ran_a_task: bool,
    wall_ms: u64,
    width: usize,
) -> String {
    let (sandbox, bad) = sb.text();
    let sandbox = paint(if bad { RED } else { GREEN }, &sandbox);

    let mut budgets = vec![
        format!("turns {}/{}", t.turns, budget.max_turns),
        format!("tools {}/{}", t.tool_calls, budget.max_tool_calls),
        format!("tok {}↑ {}↓", fmt_n(t.input_tokens), fmt_n(t.output_tokens)),
    ];
    match (t.cost_usd, budget.max_cost_usd) {
        (Some(c), Some(m)) => budgets.push(format!("cost ${}/${}", to_fixed(c, 4), to_fixed(m, 2))),
        (Some(c), None) => budgets.push(format!("cost ${}", to_fixed(c, 4))),
        (None, Some(m)) => budgets.push(format!(
            "cost unknown (limit ${} not enforced)",
            to_fixed(m, 2)
        )),
        (None, None) => {}
    }
    if let Some(ms) = budget.max_wall_ms {
        budgets.push(format!("time {}s/{}s", wall_ms / 1000, ms / 1000));
    }

    let verify = match verification {
        Some(v) => {
            let colour = match v.status.as_str() {
                "passed" => GREEN,
                "failed" => RED,
                _ => YELLOW,
            };
            let ev = if v.summary.is_empty() {
                String::new()
            } else {
                format!(": {}", short(&v.summary.replace('\n', " "), 60))
            };
            paint(colour, &format!("verify {}{ev}", v.status))
        }
        None if ran_a_task => paint(YELLOW, "verify unverified: no verification result"),
        None => paint(T_INACTIVE, "verify n/a"),
    };
    fit(
        &format!(
            " {sandbox} {} {} {verify}",
            paint(T_INACTIVE, "·"),
            paint(T_INACTIVE, &budgets.join(" · "))
        ),
        width,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use nemo_tui_base::text::strip_ansi;

    fn line(sb: &SandboxStatus, v: Option<&Verification>, task: bool, t: &Totals) -> String {
        strip_ansi(&status_line(sb, &Budget::default(), t, v, task, 0, 200))
    }

    #[test]
    fn states_are_never_flattering() {
        let t = Totals::default();
        let l = line(&SandboxStatus::default(), None, true, &t);
        assert!(l.contains("sandbox: unknown"), "{l}");
        assert!(
            l.contains("verify unverified: no verification result"),
            "{l}"
        );
        let sb = SandboxStatus {
            mode: "best-effort".into(),
            backend: "seatbelt".into(),
            filesystem: "strong".into(),
            network: "none".into(),
            unsandboxed_by_request: true,
        };
        let v = Verification {
            status: "failed".into(),
            summary: "cargo test: 2 failed".into(),
        };
        let l = line(&sb, Some(&v), true, &t);
        assert!(
            l.contains("[seatbelt fs:strong net:none] UNSANDBOXED_BY_REQUEST"),
            "{l}"
        );
        assert!(l.contains("verify failed: cargo test: 2 failed"), "{l}");
        assert!(l.contains("turns 0/50 · tools 0/200"), "{l}");
        assert!(line(&SandboxStatus::default(), None, false, &t).contains("verify n/a"));
    }

    #[test]
    fn cost_limit_without_pricing_says_so() {
        let b = Budget {
            max_cost_usd: Some(1.0),
            ..Budget::default()
        };
        let l = strip_ansi(&status_line(
            &SandboxStatus::default(),
            &b,
            &Totals::default(),
            None,
            false,
            0,
            300,
        ));
        assert!(l.contains("cost unknown (limit $1.00 not enforced)"), "{l}");
    }
}
