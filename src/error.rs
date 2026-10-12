use thiserror::Error;

#[derive(Error, Debug)]
pub enum VerifierError {
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("JSON serialization error: {0}")]
    JsonError(#[from] serde_json::Error),
    #[error("Process execution error: {0}")]
    ProcessError(String),
    #[error("Compilation/Build failed with exit code: {0}")]
    BuildFailed(i32),
    #[error("Tests failed with exit code: {0}")]
    TestsFailed(i32),
    #[error("Linter warnings or failure with exit code: {0}")]
    LinterFailed(i32),
    #[error("Scope guard violation: unauthorized modified files: {0:?}")]
    ScopeViolation(Vec<String>),
    #[error("Git error: {0}")]
    GitError(String),
    #[error("Invalid execution receipt: {0}")]
    InvalidReceipt(String),
    #[error("Runner error: {0}")]
    RunnerError(String),
    #[error("Runner not found: {0}")]
    RunnerNotFound(String),
    #[error("Diff coverage error: {0}")]
    CoverageError(String),
    #[error("Command timeout: {0}")]
    Timeout(String),
}
