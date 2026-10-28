//! Context-size bookkeeping. The provider-reported input size of the previous request is ground
//! truth; messages added since are costed with the chars/4 estimate, scaled by a ratio learned
//! from what the provider actually reported for earlier requests.

use nemo_providers::{Request, System};

/// Uncalibrated (chars/4) size of the parts of a request that are not messages: the system
/// prompt and the tool definitions.
pub fn fixed_tokens(req: &Request) -> u64 {
    let chars = |s: &str| s.chars().count() as u64;
    let system = match &req.system {
        None => 0,
        Some(System::Text(t)) => chars(t),
        Some(System::Blocks(bs)) => bs.iter().filter_map(|b| b.text.as_deref()).map(chars).sum(),
    };
    let tools: u64 = req
        .tools
        .iter()
        .map(|t| {
            chars(&t.name)
                + t.description.as_deref().map_or(0, chars)
                + chars(&t.input_schema.to_string())
        })
        .sum();
    (system + tools).div_ceil(4)
}

/// Ratio bounds: a wrong provider number must not make the estimate absurd.
const RATIO: (f64, f64) = (0.25, 4.0);

#[derive(Debug, Clone)]
pub struct ContextMeter {
    ratio: f64,
    /// `(provider-reported used tokens, raw estimate of that same request)`.
    anchor: Option<(u64, u64)>,
}

impl Default for ContextMeter {
    fn default() -> Self {
        Self {
            ratio: 1.0,
            anchor: None,
        }
    }
}

impl ContextMeter {
    /// Calibrated size for a request whose raw estimate is `raw`.
    pub fn estimate(&self, raw: u64) -> u64 {
        let scaled = match self.anchor {
            Some((used, anchor_raw)) => used as f64 + self.ratio * (raw as f64 - anchor_raw as f64),
            None => self.ratio * raw as f64,
        };
        scaled.max(0.0).round() as u64
    }

    /// The provider reported `used` input tokens for a request whose raw estimate was `raw`.
    pub fn observe(&mut self, raw: u64, used: u64) {
        if raw > 0 && used > 0 {
            self.ratio = (used as f64 / raw as f64).clamp(RATIO.0, RATIO.1);
            self.anchor = Some((used, raw));
        }
    }

    /// The history was rewritten: the last provider number no longer describes it.
    pub fn forget_anchor(&mut self) {
        self.anchor = None;
    }

    /// Last provider-reported size, while it still describes the history.
    pub fn provider_used(&self) -> Option<u64> {
        self.anchor.map(|(u, _)| u)
    }

    pub fn ratio(&self) -> f64 {
        self.ratio
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calibrates_from_provider_usage_and_anchors_on_it() {
        let mut m = ContextMeter::default();
        assert_eq!(m.estimate(1000), 1000, "uncalibrated = chars/4");
        m.observe(1000, 1500); // provider counts 1.5x what chars/4 says
        assert!((m.ratio() - 1.5).abs() < 1e-9);
        assert_eq!(m.estimate(1000), 1500, "same request: ground truth");
        assert_eq!(m.estimate(1200), 1800, "growth is scaled by the ratio");
        assert_eq!(m.estimate(800), 1200, "shrinkage too");
        m.forget_anchor();
        assert_eq!(m.estimate(400), 600, "ratio survives, anchor does not");
        assert_eq!(m.provider_used(), None);
        m.observe(10, 1_000_000);
        assert_eq!(m.ratio(), 4.0, "clamped");
        m.observe(0, 5);
        assert_eq!(m.ratio(), 4.0, "unusable observation ignored");
    }
}
