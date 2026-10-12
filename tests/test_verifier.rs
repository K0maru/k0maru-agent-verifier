use async_trait::async_trait;
use k0maru_verifier::error::VerifierError;
use k0maru_verifier::receipt::{
    CompilationCheck, ExecutionReceipt, LinterCheck, StepStatus, TestCheck, VerdictStatus,
};
use k0maru_verifier::runner::{LanguageRunner, RunnerRegistry};
use k0maru_verifier::verifier::{VerifierConfig, WorkspaceVerifier};
use std::path::Path;
use std::sync::Arc;
use tempfile::TempDir;

/// Helper to set up an initialized git repository with an initial commit.
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

    // Initial commit
    std::fs::write(root.join("README.md"), "# Verifier Test Workspace\n").unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        "pub fn add(a: i32, b: i32) -> i32 { a + b }\n",
    )
    .unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();

    run_git(&["add", "."]);
    run_git(&["commit", "-m", "Initial commit"]);

    temp
}

/// A configurable mock language runner to deterministically test verifier execution states.
#[derive(Clone)]
struct TestMockRunner {
    name: &'static str,
    compilation: CompilationCheck,
    tests: TestCheck,
    linter: LinterCheck,
}

#[async_trait]
impl LanguageRunner for TestMockRunner {
    fn name(&self) -> &'static str {
        self.name
    }

    fn detect(&self, _workspace: &Path) -> bool {
        true
    }

    async fn check(&self, _workspace: &Path) -> Result<CompilationCheck, VerifierError> {
        Ok(self.compilation.clone())
    }

    async fn test(&self, _workspace: &Path) -> Result<TestCheck, VerifierError> {
        Ok(self.tests.clone())
    }

    async fn lint(&self, _workspace: &Path) -> Result<LinterCheck, VerifierError> {
        Ok(self.linter.clone())
    }
}

fn passing_mock_runner() -> TestMockRunner {
    TestMockRunner {
        name: "mock-runner",
        compilation: CompilationCheck {
            status: StepStatus::Passed,
            exit_code: 0,
            message: None,
        },
        tests: TestCheck {
            status: StepStatus::Passed,
            exit_code: Some(0),
            total: 10,
            failed: 0,
            duration_ms: Some(150),
            message: None,
        },
        linter: LinterCheck {
            status: StepStatus::Passed,
            warnings: 0,
            exit_code: Some(0),
            message: None,
        },
    }
}

// ============================================================================
// 1. Successful Verification Flow
// ============================================================================

#[tokio::test]
async fn test_verify_successful_flow() {
    let repo = setup_temp_git_workspace();
    let mut registry = RunnerRegistry::empty();
    registry.register(Box::new(passing_mock_runner()));

    let verifier = WorkspaceVerifier::with_registry(Arc::new(registry));
    let config =
        VerifierConfig::new(repo.path(), "TASK-VERIFY-OK").whitelist(vec!["src/".to_string()]);

    let receipt = verifier
        .verify(&config)
        .await
        .expect("verify should succeed");

    assert_eq!(receipt.verdict, VerdictStatus::Success);
    assert!(receipt.is_passing());
    assert_eq!(receipt.task_id, "TASK-VERIFY-OK");

    // All checks passed
    assert!(receipt.checks.compilation.is_passed());
    assert!(receipt.checks.tests.is_passed());
    assert!(receipt.checks.linter.is_passed());
    assert!(receipt.checks.scope_guard.is_passed());

    // Artifacts present
    assert!(receipt.artifacts.git_patch_sha256.is_some());
    assert!(receipt.artifacts.receipt_timestamp.is_some());

    // Schema validation succeeds
    assert!(receipt.validate().is_ok());
}

// ============================================================================
// 2. Failure when Tests Fail
// ============================================================================

#[tokio::test]
async fn test_verify_failure_when_test_fails() {
    let repo = setup_temp_git_workspace();
    let mut runner = passing_mock_runner();
    runner.tests = TestCheck {
        status: StepStatus::Failed,
        exit_code: Some(101),
        total: 8,
        failed: 3,
        duration_ms: Some(210),
        message: Some("3 tests failed: test_a, test_b, test_c".to_string()),
    };

    let mut registry = RunnerRegistry::empty();
    registry.register(Box::new(runner));

    let verifier = WorkspaceVerifier::with_registry(Arc::new(registry));
    let config = VerifierConfig::new(repo.path(), "TASK-TEST-FAIL");

    let receipt = verifier
        .verify(&config)
        .await
        .expect("verify execution completes");

    assert_eq!(receipt.verdict, VerdictStatus::Failed);
    assert!(!receipt.is_passing());
    assert_eq!(receipt.checks.tests.status, StepStatus::Failed);
    assert_eq!(receipt.checks.tests.failed, 3);
    assert_eq!(receipt.checks.tests.total, 8);
    assert!(receipt.validate().is_ok());
}

// ============================================================================
// 3. Failure when Compilation Fails
// ============================================================================

#[tokio::test]
async fn test_verify_failure_when_compilation_fails() {
    let repo = setup_temp_git_workspace();
    let mut runner = passing_mock_runner();
    runner.compilation = CompilationCheck {
        status: StepStatus::Failed,
        exit_code: 1,
        message: Some("error[E0425]: cannot find value `foo` in this scope".to_string()),
    };

    let mut registry = RunnerRegistry::empty();
    registry.register(Box::new(runner));

    let verifier = WorkspaceVerifier::with_registry(Arc::new(registry));
    let config = VerifierConfig::new(repo.path(), "TASK-COMP-FAIL");

    let receipt = verifier
        .verify(&config)
        .await
        .expect("verify execution completes");

    assert_eq!(receipt.verdict, VerdictStatus::Failed);
    assert!(!receipt.is_passing());
    assert_eq!(receipt.checks.compilation.status, StepStatus::Failed);
    assert_eq!(receipt.checks.compilation.exit_code, 1);
    assert!(receipt.validate().is_ok());
}

// ============================================================================
// 4. Failure when Linter Reports Warnings
// ============================================================================

#[tokio::test]
async fn test_verify_failure_when_linter_warns() {
    let repo = setup_temp_git_workspace();
    let mut runner = passing_mock_runner();
    runner.linter = LinterCheck {
        status: StepStatus::Failed,
        warnings: 2,
        exit_code: Some(1),
        message: Some("2 clippy warnings found".to_string()),
    };

    let mut registry = RunnerRegistry::empty();
    registry.register(Box::new(runner));

    let verifier = WorkspaceVerifier::with_registry(Arc::new(registry));
    let config = VerifierConfig::new(repo.path(), "TASK-LINT-FAIL");

    let receipt = verifier
        .verify(&config)
        .await
        .expect("verify execution completes");

    assert_eq!(receipt.verdict, VerdictStatus::Failed);
    assert!(!receipt.is_passing());
    assert_eq!(receipt.checks.linter.status, StepStatus::Failed);
    assert_eq!(receipt.checks.linter.warnings, 2);
    assert!(receipt.validate().is_ok());
}

// ============================================================================
// 5. Failure when Scope Guard is Violated
// ============================================================================

#[tokio::test]
async fn test_verify_failure_when_scope_guard_violated() {
    let repo = setup_temp_git_workspace();
    let root = repo.path();

    // Create unauthorized file outside whitelist ["src/"]
    std::fs::write(root.join("unauthorized_script.py"), "print('unauthorized')").unwrap();

    let mut registry = RunnerRegistry::empty();
    registry.register(Box::new(passing_mock_runner()));

    let verifier = WorkspaceVerifier::with_registry(Arc::new(registry));
    let config = VerifierConfig::new(root, "TASK-SCOPE-FAIL").whitelist(vec!["src/".to_string()]);

    let receipt = verifier
        .verify(&config)
        .await
        .expect("verify execution completes");

    assert_eq!(receipt.verdict, VerdictStatus::Failed);
    assert!(!receipt.is_passing());
    assert_eq!(receipt.checks.scope_guard.status, StepStatus::Failed);
    assert!(receipt
        .checks
        .scope_guard
        .unauthorized_files
        .contains(&"unauthorized_script.py".to_string()));
    assert!(receipt.validate().is_ok());
}

// ============================================================================
// 6. Blocked when No Runner is Detected
// ============================================================================

#[tokio::test]
async fn test_verify_blocked_when_no_runner_detected() {
    let repo = setup_temp_git_workspace();
    // Empty registry with no runners
    let registry = RunnerRegistry::empty();

    let verifier = WorkspaceVerifier::with_registry(Arc::new(registry));
    let config = VerifierConfig::new(repo.path(), "TASK-BLOCKED");

    let receipt = verifier
        .verify(&config)
        .await
        .expect("verify execution completes");

    assert_eq!(receipt.verdict, VerdictStatus::Blocked);
    assert!(!receipt.is_passing());
    assert_eq!(receipt.checks.compilation.status, StepStatus::Skipped);
    assert_eq!(receipt.checks.tests.status, StepStatus::Skipped);
    assert_eq!(receipt.checks.linter.status, StepStatus::Skipped);
    assert!(receipt.validate().is_ok());
}

// ============================================================================
// 7. Verify Patch SHA-256 Matches Actual `git diff HEAD`
// ============================================================================

#[tokio::test]
async fn test_verify_patch_sha256_matches_git_diff_head() {
    let repo = setup_temp_git_workspace();
    let root = repo.path();

    // Make concrete changes to tracked file
    let addition = "\npub fn multiply(a: i32, b: i32) -> i32 { a * b }\n";
    let mut lib_content = std::fs::read_to_string(root.join("src/lib.rs")).unwrap();
    lib_content.push_str(addition);
    std::fs::write(root.join("src/lib.rs"), lib_content).unwrap();

    let mut registry = RunnerRegistry::empty();
    registry.register(Box::new(passing_mock_runner()));

    let verifier = WorkspaceVerifier::with_registry(Arc::new(registry));
    let config = VerifierConfig::new(root, "TASK-SHA256-CHECK").whitelist(vec!["src/".to_string()]);

    let receipt = verifier.verify(&config).await.expect("verify succeeds");

    // Manually run git diff HEAD in the workspace
    let git_diff_output = std::process::Command::new("git")
        .args(["diff", "HEAD"])
        .current_dir(root)
        .output()
        .expect("git diff HEAD");
    assert!(git_diff_output.status.success());

    let actual_diff_bytes = &git_diff_output.stdout;
    assert!(!actual_diff_bytes.is_empty(), "diff should not be empty");

    let expected_hash = ExecutionReceipt::compute_sha256(actual_diff_bytes);
    let receipt_hash = receipt
        .artifacts
        .git_patch_sha256
        .as_ref()
        .expect("patch sha256 must be present");

    assert_eq!(receipt_hash, &expected_hash);
    assert!(receipt.verify_patch_sha256(actual_diff_bytes).unwrap());
}

// ============================================================================
// 8. Error Handling Edge Cases
// ============================================================================

#[tokio::test]
async fn test_verify_non_existent_workspace_error() {
    let verifier = WorkspaceVerifier::new();
    let config = VerifierConfig::new("/non_existent_directory_99999", "TASK-ERR");

    let err = verifier
        .verify(&config)
        .await
        .expect_err("should fail with IoError");
    assert!(matches!(err, VerifierError::IoError(_)));
}

#[tokio::test]
async fn test_verify_runner_override_not_found_error() {
    let repo = setup_temp_git_workspace();
    let verifier = WorkspaceVerifier::new();
    let config = VerifierConfig::new(repo.path(), "TASK-ERR")
        .override_runner("non_existent_runner_name".to_string());

    let err = verifier
        .verify(&config)
        .await
        .expect_err("should fail with RunnerNotFound");
    match err {
        VerifierError::RunnerNotFound(msg) => {
            assert!(msg.contains("non_existent_runner_name"));
        }
        other => panic!("Expected RunnerNotFound, got {other:?}"),
    }
}
