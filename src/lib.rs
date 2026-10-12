//! K0maru Agent Verifier: Deterministic quality gate and execution receipt signing engine.

pub mod constants;
pub mod error;
pub mod guard;
pub mod receipt;
pub mod review;
pub mod runner;
pub mod verifier;

pub use constants::*;
pub use error::VerifierError;
pub use guard::ScopeGuard;
pub use receipt::{
    resolve_receipts_dir, CheckSummary, CompilationCheck, ExecutionReceipt, LinterCheck,
    ScopeGuardCheck, ScopeGuardResult, StepResult, StepStatus, TestCheck, TestStepResult,
    VerdictStatus,
};
pub use review::ReviewPack;
pub use runner::{LanguageRunner, RunnerRegistry};
pub use verifier::{VerifierConfig, WorkspaceVerifier};
