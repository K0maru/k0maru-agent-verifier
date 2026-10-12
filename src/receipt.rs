use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

pub const DEFAULT_RECEIPT_SCHEMA: &str = "https://k0maru.dev/schemas/agent-receipt-v1.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionReceipt {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub task_id: String,
    pub workspace: PathBuf,
    pub verdict: VerdictStatus,
    pub checks: CheckSummary,
    pub artifacts: ArtifactSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerdictStatus {
    Success,
    Failed,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckSummary {
    pub compilation: StepResult,
    pub tests: TestStepResult,
    pub linter: StepResult,
    pub scope_guard: ScopeGuardResult,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    pub status: StepStatus,
    pub exit_code: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Passed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestStepResult {
    pub status: StepStatus,
    pub exit_code: i32,
    pub total: usize,
    pub failed: usize,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScopeGuardResult {
    pub status: StepStatus,
    pub modified_files: Vec<String>,
    pub unauthorized_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactSummary {
    pub git_patch_sha256: String,
    pub receipt_timestamp: String,
}

impl ExecutionReceipt {
    pub fn compute_sha256(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        hex::encode(hasher.finalize())
    }
}
