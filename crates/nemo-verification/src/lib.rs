//! Verification engine (docs/specs/verification.md): check planning, execution through
//! `nemo-terminal`, status aggregation, completion gate and bounded self-correction.

mod gate;
mod plan;
mod runner;
mod types;

pub use gate::*;
pub use plan::{detect_checks, plan_checks};
pub use runner::VerifyRunner;
pub use types::*;
