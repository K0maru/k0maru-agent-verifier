//! K0maru Agent Verifier: Deterministic quality gate and execution receipt signing engine.

pub mod constants;
pub mod coverage;
pub mod error;
pub mod falsification;
pub mod guard;
pub mod kdsl;
pub mod receipt;
pub mod review;
pub mod runner;
pub mod tuple;
pub mod verifier;

pub use constants::*;
pub use coverage::{CoverageReport, DiffCoverageResult, FileDiffLines};
pub use error::VerifierError;
pub use falsification::run_red_check;
pub use guard::ScopeGuard;
pub use kdsl::{KdslInvariant, KdslVerifyClause};
pub use receipt::{
    resolve_receipts_dir, CheckSummary, CompilationCheck, DiffCoverageCheck, ExecutionReceipt,
    LinterCheck, RedCheckResult, ScopeGuardCheck, ScopeGuardResult, StepResult, StepStatus,
    TestCheck, TestStepResult, VerdictStatus,
};
pub use review::ReviewPack;
pub use runner::{LanguageRunner, RunnerRegistry};
pub use tuple::{resolve_tuples_dir, LindaTuple, TupleEmitter, VerificationTuplePayload};
pub use verifier::{VerifierConfig, WorkspaceVerifier};
