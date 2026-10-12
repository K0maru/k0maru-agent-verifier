//! K0maru Agent Verifier: Deterministic quality gate and execution receipt signing engine.

pub mod error;
pub mod guard;
pub mod receipt;
pub mod runner;
pub mod verifier;

pub use error::VerifierError;
pub use guard::ScopeGuard;
pub use receipt::{ExecutionReceipt, StepStatus, VerdictStatus};
pub use runner::{LanguageRunner, RunnerRegistry};
pub use verifier::{VerifierConfig, WorkspaceVerifier};
