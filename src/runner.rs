use crate::error::VerifierError;
use crate::receipt::{StepResult, StepStatus, TestStepResult};
use async_trait::async_trait;
use std::path::Path;
use std::time::Instant;
use tokio::process::Command;

#[async_trait]
pub trait LanguageRunner: Send + Sync {
    fn detect(&self, workspace: &Path) -> bool;
    async fn check(&self, workspace: &Path) -> Result<StepResult, VerifierError>;
    async fn test(&self, workspace: &Path) -> Result<TestStepResult, VerifierError>;
    async fn lint(&self, workspace: &Path) -> Result<StepResult, VerifierError>;
}

pub struct CargoRunner;

#[async_trait]
impl LanguageRunner for CargoRunner {
    fn detect(&self, workspace: &Path) -> bool {
        workspace.join("Cargo.toml").exists()
    }

    async fn check(&self, workspace: &Path) -> Result<StepResult, VerifierError> {
        let output = Command::new("cargo")
            .arg("check")
            .current_dir(workspace)
            .output()
            .await?;

        let exit_code = output.status.code().unwrap_or(-1);
        let status = if exit_code == 0 { StepStatus::Passed } else { StepStatus::Failed };
        let msg = if exit_code != 0 {
            Some(String::from_utf8_lossy(&output.stderr).to_string())
        } else {
            None
        };

        Ok(StepResult { status, exit_code, message: msg })
    }

    async fn test(&self, workspace: &Path) -> Result<TestStepResult, VerifierError> {
        let start = Instant::now();
        let output = Command::new("cargo")
            .arg("test")
            .current_dir(workspace)
            .output()
            .await?;

        let duration_ms = start.elapsed().as_millis() as u64;
        let exit_code = output.status.code().unwrap_or(-1);
        let status = if exit_code == 0 { StepStatus::Passed } else { StepStatus::Failed };

        Ok(TestStepResult {
            status,
            exit_code,
            total: 0,
            failed: if exit_code == 0 { 0 } else { 1 },
            duration_ms,
        })
    }

    async fn lint(&self, workspace: &Path) -> Result<StepResult, VerifierError> {
        let output = Command::new("cargo")
            .args(["clippy", "--all-targets", "--", "-D", "warnings"])
            .current_dir(workspace)
            .output()
            .await?;

        let exit_code = output.status.code().unwrap_or(-1);
        let status = if exit_code == 0 { StepStatus::Passed } else { StepStatus::Failed };
        let msg = if exit_code != 0 {
            Some(String::from_utf8_lossy(&output.stderr).to_string())
        } else {
            None
        };

        Ok(StepResult { status, exit_code, message: msg })
    }
}
