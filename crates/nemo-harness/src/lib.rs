//! Agent harness (R4e): the model/tool loop, stop conditions, session/task state machine,
//! sub-agents and the completion gate hand-off. A library: interfaces plug in through the
//! [`Approver`], [`InputProvider`] and [`ContextProvider`] traits (headless defaults included).
//!
//! The model's final message never completes a task: it is a proposal that goes through
//! `CompletionGate::evaluate` (design doc §6.2).

mod agent;
mod agents;
mod context;
mod exec;
mod harness;
mod sink;
pub mod testing;
mod toolbox;
mod traits;
mod util;

pub use agents::{
    AgentDef, AgentSource, AgentTable, GENERAL_PURPOSE, from_mode, stricter, to_mode,
};
pub use context::{RULES, StaticContext, wrap_tool_output};
pub use harness::{
    GateFactory, Harness, HarnessConfig, HarnessError, Pricing, TaskOutcome, TaskRequest,
    browser_config, budget_from_settings,
};
pub use sink::{EventSink, Scope};
pub use toolbox::{ToolBox, validate};
pub use traits::*;
pub use util::{is_invisible_or_blank, sanitize_for_display};
