//! Integration tests for Phase 4: Flaky Test Quarantine.

use async_trait::async_trait;
use k0maru_verifier::receipt::*;
use k0maru_verifier::runner::{LanguageRunner, RunnerRegistry};
use k0maru_verifier::verifier::{VerifierConfig, WorkspaceVerifier};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tempfile::TempDir;

struct FlakyMockRunner {
    call_count: AtomicUsize,
    fail_until_attempt: usize,
}

impl FlakyMockRunner {
    fn new(fail_until_attempt: usize) -> Self {
        Self {
            call_count: AtomicUsize::new(0),
            fail_until_attempt,
        }
    }
}

#[async_trait]
impl LanguageRunner for FlakyMockRunner {
    fn name(&self) -> &'static str {
        "flaky-mock"
    }

    fn detect(&self, _workspace: &Path) -> bool {
        true
    }

    async fn check(
        &self,
        _workspace: &Path,
    ) -> Result<CompilationCheck, k0maru_verifier::VerifierError> {
        Ok(CompilationCheck {
            status: StepStatus::Passed,
            exit_code: 0,
            message: None,
        })
    }

    async fn test(&self, _workspace: &Path) -> Result<TestCheck, k0maru_verifier::VerifierError> {
        let current = self.call_count.fetch_add(1, Ordering::SeqCst) + 1;
        if current <= self.fail_until_attempt {
            // Transient failure
            Ok(TestCheck {
                status: StepStatus::Failed,
                exit_code: Some(101),
                total: 5,
                failed: 1,
                duration_ms: Some(15),
                message: Some(format!(
                    "Flaky network assertion timeout (attempt #{current})"
                )),
                flaky_quarantined: None,
                retries: None,
            })
        } else {
            // Passed upon retry!
            Ok(TestCheck {
                status: StepStatus::Passed,
                exit_code: Some(0),
                total: 5,
                failed: 0,
                duration_ms: Some(12),
                message: None,
                flaky_quarantined: None,
                retries: None,
            })
        }
    }

    async fn lint(&self, _workspace: &Path) -> Result<LinterCheck, k0maru_verifier::VerifierError> {
        Ok(LinterCheck {
            status: StepStatus::Passed,
            warnings: 0,
            exit_code: Some(0),
            message: None,
        })
    }
}

fn setup_temp_git_workspace() -> TempDir {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let run_git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .expect("execute git command");
        assert!(
            output.status.success(),
            "Git command failed: {:?}, stderr: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    };

    run_git(&["init"]);
    run_git(&["config", "user.name", "K0maru Verifier Tester"]);
    run_git(&["config", "user.email", "verifier@k0maru.dev"]);
    run_git(&["config", "commit.gpgsign", "false"]);

    std::fs::write(root.join("README.md"), "# Test Workspace\n").unwrap();
    run_git(&["add", "."]);
    run_git(&["commit", "-m", "Initial commit"]);

    temp
}

#[tokio::test]
async fn test_flaky_test_fails_without_retry() {
    let temp = setup_temp_git_workspace();
    let runner = FlakyMockRunner::new(1); // Fails on 1st run, would pass on 2nd

    let mut registry = RunnerRegistry::empty();
    registry.register(Box::new(runner));
    let verifier = WorkspaceVerifier::with_registry(Arc::new(registry));

    let config = VerifierConfig::new(temp.path(), "task-no-retry").flaky_retry(0);

    let receipt = verifier.verify(&config).await.expect("verify completes");
    assert_eq!(receipt.verdict, VerdictStatus::Failed);
    assert!(!receipt.is_passing());
    assert_eq!(receipt.checks.tests.status, StepStatus::Failed);
    assert_eq!(receipt.checks.tests.failed, 1);
    assert_eq!(receipt.checks.tests.flaky_quarantined, None);
}

#[tokio::test]
async fn test_flaky_test_quarantine_succeeds_with_retry() {
    let temp = setup_temp_git_workspace();
    let runner = FlakyMockRunner::new(1); // Fails on 1st run, passes on retry #1

    let mut registry = RunnerRegistry::empty();
    registry.register(Box::new(runner));
    let verifier = WorkspaceVerifier::with_registry(Arc::new(registry));

    let config = VerifierConfig::new(temp.path(), "task-flaky-retry").flaky_retry(2);

    let receipt = verifier.verify(&config).await.expect("verify completes");
    assert_eq!(receipt.verdict, VerdictStatus::Success);
    assert!(receipt.is_passing());

    let test_check = &receipt.checks.tests;
    assert_eq!(test_check.status, StepStatus::Passed);
    assert_eq!(test_check.failed, 0);
    assert_eq!(test_check.flaky_quarantined, Some(true));
    assert_eq!(test_check.retries, Some(1));
    assert!(test_check
        .message
        .as_ref()
        .unwrap()
        .contains("quarantined flaky test"));

    let summary = receipt.summary();
    assert!(summary.contains("[quarantined after 1 retries]"));
}

#[tokio::test]
async fn test_persistent_failure_fails_after_exhausting_retries() {
    let temp = setup_temp_git_workspace();
    let runner = FlakyMockRunner::new(5); // Fails 5 times

    let mut registry = RunnerRegistry::empty();
    registry.register(Box::new(runner));
    let verifier = WorkspaceVerifier::with_registry(Arc::new(registry));

    let config = VerifierConfig::new(temp.path(), "task-persistent-fail").flaky_retry(2); // Only retry up to 2 times

    let receipt = verifier.verify(&config).await.expect("verify completes");
    assert_eq!(receipt.verdict, VerdictStatus::Failed);
    assert!(!receipt.is_passing());
    assert_eq!(receipt.checks.tests.status, StepStatus::Failed);
    assert_eq!(receipt.checks.tests.flaky_quarantined, None);
}
