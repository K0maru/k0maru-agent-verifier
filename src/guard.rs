use crate::error::VerifierError;
use crate::receipt::{ScopeGuardResult, StepStatus};
use std::path::Path;
use tokio::process::Command;

pub struct ScopeGuard;

impl ScopeGuard {
    pub async fn check_scope(
        workspace: &Path,
        allowed_whitelist: Option<&[String]>,
    ) -> Result<ScopeGuardResult, VerifierError> {
        let output = Command::new("git")
            .args(["diff", "--name-only", "HEAD"])
            .current_dir(workspace)
            .output()
            .await?;

        let text = String::from_utf8_lossy(&output.stdout);
        let modified_files: Vec<String> = text
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let mut unauthorized = Vec::new();
        if let Some(whitelist) = allowed_whitelist {
            for file in &modified_files {
                if !whitelist.iter().any(|allowed| file.starts_with(allowed)) {
                    unauthorized.push(file.clone());
                }
            }
        }

        let status = if unauthorized.is_empty() {
            StepStatus::Passed
        } else {
            StepStatus::Failed
        };

        Ok(ScopeGuardResult {
            status,
            modified_files,
            unauthorized_files: unauthorized,
        })
    }
}
