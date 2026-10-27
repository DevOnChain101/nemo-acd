//! MCP client manager on the official `rmcp` crate (stdio child process + streamable HTTP).

mod config;
mod manager;
mod naming;
mod result;

pub use config::*;
pub use manager::*;
pub use naming::{MAX_TOOL_NAME_LEN, parse_tool_name_fallback, tool_name_for};
pub use result::{McpContent, McpToolResult, normalize_result};
