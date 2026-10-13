use crate::error::VerifierError;
use crate::guard::ScopeGuard;
use crate::receipt::{
    ArtifactSummary, CheckSummary, CompilationCheck, ExecutionReceipt, LinterCheck,
    ScopeGuardCheck, StepStatus, TestCheck, VerdictStatus, DEFAULT_RECEIPT_SCHEMA,
};
use crate::runner::{LanguageRunner, RunnerRegistry};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::process::Command;

/// Configuration for running workspace verification.
#[derive(Debug, Clone)]
pub struct VerifierConfig {
    /// Target workspace directory.
    pub workspace: PathBuf,
    /// Unique task identifier for receipt signing.
    pub task_id: String,
    /// Whitelist patterns restricting modified/added/untracked files.
    pub whitelist: Option<Vec<String>>,
    /// Explicit language runner override (e.g. "cargo", "pytest", "npm").
    pub override_runner: Option<String>,
    /// Whether to auto-detect runner if override is not provided (default: true).
    pub auto_detect: bool,
    /// Base branch for immutable test baseline comparison (e.g. "main", "dev").
    pub base_branch: Option<String>,
    /// Whether to enforce immutable test protection preventing deletion of existing test lines.
    pub immutable_tests: bool,
    /// Minimum required test coverage percentage on newly added/modified diff lines.
    pub min_diff_coverage: Option<f64>,
    /// Optional path to coverage report file (LCOV or JSON).
    pub coverage_file: Option<PathBuf>,
    /// Maximum retry attempts for transient flaky test failures.
    pub flaky_retry: usize,
    /// Optional K-DSL dynamic verification directive clause.
    pub kdsl: Option<crate::kdsl::KdslVerifyClause>,
}

impl VerifierConfig {
    /// Creates a new `VerifierConfig` with the given workspace and task ID.
    pub fn new(workspace: impl Into<PathBuf>, task_id: impl Into<String>) -> Self {
        Self {
            workspace: workspace.into(),
            task_id: task_id.into(),
            whitelist: None,
            override_runner: None,
            auto_detect: true,
            base_branch: None,
            immutable_tests: false,
            min_diff_coverage: None,
            coverage_file: None,
            flaky_retry: 0,
            kdsl: None,
        }
    }

    /// Sets the target workspace directory.
    pub fn workspace(mut self, workspace: impl Into<PathBuf>) -> Self {
        self.workspace = workspace.into();
        self
    }

    /// Sets the task identifier.
    pub fn task_id(mut self, task_id: impl Into<String>) -> Self {
        self.task_id = task_id.into();
        self
    }

    /// Sets the allowed files whitelist.
    pub fn whitelist(mut self, whitelist: impl Into<Option<Vec<String>>>) -> Self {
        self.whitelist = whitelist.into();
        self
    }

    /// Overrides the language runner to use by name.
    pub fn override_runner(mut self, runner: impl Into<Option<String>>) -> Self {
        self.override_runner = runner.into();
        self
    }

    /// Enables or disables runner auto-detection.
    pub fn auto_detect(mut self, auto_detect: bool) -> Self {
        self.auto_detect = auto_detect;
        self
    }

    /// Sets the base branch for immutable test protection.
    pub fn base_branch(mut self, base_branch: impl Into<Option<String>>) -> Self {
        self.base_branch = base_branch.into();
        self
    }

    /// Enables or disables immutable test baseline protection.
    pub fn immutable_tests(mut self, immutable_tests: bool) -> Self {
        self.immutable_tests = immutable_tests;
        self
    }

    /// Sets minimum required diff coverage threshold percentage.
    pub fn min_diff_coverage(mut self, min: impl Into<Option<f64>>) -> Self {
        self.min_diff_coverage = min.into();
        self
    }

    /// Sets explicit coverage report file path.
    pub fn coverage_file(mut self, file: impl Into<Option<PathBuf>>) -> Self {
        self.coverage_file = file.into();
        self
    }

    /// Sets maximum retry attempts for flaky test quarantine.
    pub fn flaky_retry(mut self, retries: usize) -> Self {
        self.flaky_retry = retries;
        self
    }

    /// Sets the parsed K-DSL verification directive clause.
    pub fn kdsl(mut self, kdsl: impl Into<Option<crate::kdsl::KdslVerifyClause>>) -> Self {
        self.kdsl = kdsl.into();
        self
    }
}

impl Default for VerifierConfig {
    fn default() -> Self {
        Self {
            workspace: PathBuf::from("."),
            task_id: "task_ad_hoc".to_string(),
            whitelist: None,
            override_runner: None,
            auto_detect: true,
            base_branch: None,
            immutable_tests: false,
            min_diff_coverage: None,
            coverage_file: None,
            flaky_retry: 0,
            kdsl: None,
        }
    }
}

/// Central coordinator for running multi-stage quality verification on a workspace.
pub struct WorkspaceVerifier {
    registry: Arc<RunnerRegistry>,
}

impl WorkspaceVerifier {
    /// Creates a new `WorkspaceVerifier` with the default runner registry.
    pub fn new() -> Self {
        Self {
            registry: Arc::new(RunnerRegistry::new()),
        }
    }

    /// Creates a `WorkspaceVerifier` using a custom runner registry.
    pub fn with_registry(registry: Arc<RunnerRegistry>) -> Self {
        Self { registry }
    }

    /// Returns a reference to the active `RunnerRegistry`.
    pub fn registry(&self) -> &RunnerRegistry {
        &self.registry
    }

    /// Executes the full verification pipeline and signs an `ExecutionReceipt`.
    ///
    /// Pipeline steps:
    /// 1. Resolve and validate workspace directory.
    /// 2. Detect language runner via `RunnerRegistry` or honor explicit override.
    /// 3. Execute compilation check (`runner.check`).
    /// 4. Execute test suite check (`runner.test`).
    /// 5. Execute static analysis / linter check (`runner.lint`).
    /// 6. Execute scope guard check (`ScopeGuard::check_scope`).
    /// 7. Compute Git diff patch SHA-256 fingerprint (`git diff HEAD`).
    /// 8. Formulate overall `VerdictStatus` (Success / Failed / Blocked).
    /// 9. Assemble `ExecutionReceipt` and validate schema invariants.
    pub async fn verify(&self, config: &VerifierConfig) -> Result<ExecutionReceipt, VerifierError> {
        let workspace = if config.workspace.exists() {
            std::fs::canonicalize(&config.workspace)?
        } else {
            return Err(VerifierError::IoError(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!(
                    "Workspace path does not exist: {}",
                    config.workspace.display()
                ),
            )));
        };

        // 1. Runner detection
        let selected_runner: Option<&dyn LanguageRunner> =
            if let Some(ref name) = config.override_runner {
                match self.registry.get_by_name(name) {
                    Some(r) => Some(r),
                    None => {
                        return Err(VerifierError::RunnerNotFound(format!(
                            "Runner '{name}' not found in registry (available: {:?})",
                            self.registry.runner_names()
                        )));
                    }
                }
            } else if config.auto_detect {
                self.registry.detect_runner(&workspace)
            } else {
                None
            };

        let mut is_blocked = false;

        // 2, 3, 4. Compilation, tests, and lint checks
        let (compilation, tests, linter) = match selected_runner {
            Some(runner) => {
                let comp = runner.check(&workspace).await?;

                let mut t = runner.test(&workspace).await?;
                let mut retry_count = 0;
                while !t.is_passed() && retry_count < config.flaky_retry {
                    retry_count += 1;
                    let retry_res = runner.test(&workspace).await?;
                    if retry_res.is_passed() {
                        let initial_failed = t.failed;
                        t = TestCheck {
                            status: StepStatus::Passed,
                            exit_code: Some(0),
                            total: retry_res.total,
                            failed: 0,
                            duration_ms: retry_res.duration_ms,
                            message: Some(format!(
                                "Passed after retry #{}/{} (quarantined flaky test; initial run had {} failures)",
                                retry_count, config.flaky_retry, initial_failed
                            )),
                            flaky_quarantined: Some(true),
                            retries: Some(retry_count),
                        };
                        break;
                    } else {
                        t = retry_res;
                    }
                }

                let lint = runner.lint(&workspace).await?;
                (comp, t, lint)
            }
            None => {
                is_blocked = true;
                let msg = format!(
                    "No supported language runner detected for workspace: {}",
                    workspace.display()
                );
                (
                    CompilationCheck {
                        status: StepStatus::Skipped,
                        exit_code: 0,
                        message: Some(msg.clone()),
                    },
                    TestCheck {
                        status: StepStatus::Skipped,
                        exit_code: None,
                        total: 0,
                        failed: 0,
                        duration_ms: None,
                        message: Some(msg.clone()),
                        flaky_quarantined: None,
                        retries: None,
                    },
                    LinterCheck {
                        status: StepStatus::Skipped,
                        warnings: 0,
                        exit_code: None,
                        message: Some(msg),
                    },
                )
            }
        };

        let immutable_tests = config.immutable_tests
            || config
                .kdsl
                .as_ref()
                .map(|k| k.has_immutable_tests())
                .unwrap_or(false);
        let base_branch = config
            .base_branch
            .as_deref()
            .or_else(|| config.kdsl.as_ref().and_then(|k| k.immutable_base_branch()));

        // 5. Scope Guard check
        let scope_guard = match ScopeGuard::check_scope_extended(
            &workspace,
            config.whitelist.as_deref(),
            base_branch,
            immutable_tests,
        )
        .await
        {
            Ok(check) => check,
            Err(VerifierError::GitError(err)) => {
                is_blocked = true;
                ScopeGuardCheck {
                    status: StepStatus::Skipped,
                    modified_files: Vec::new(),
                    unauthorized_files: vec![format!("Git error: {err}")],
                    message: Some(format!("Git error: {err}")),
                }
            }
            Err(e) => return Err(e),
        };

        // 6. Diff coverage check (if requested)
        let diff_coverage = if config.min_diff_coverage.is_some() || config.coverage_file.is_some()
        {
            crate::coverage::check_diff_coverage(
                &workspace,
                config.min_diff_coverage,
                config.coverage_file.as_deref(),
                config.base_branch.as_deref(),
            )
            .await?
        } else {
            None
        };

        // 7. Compute Git Patch SHA-256 fingerprint
        let git_patch_sha256 = compute_git_diff_sha256(&workspace).await;

        // 8. Formulate Verdict
        let checks_all_passed = compilation.is_passed()
            && tests.is_passed()
            && linter.is_passed()
            && scope_guard.is_passed()
            && diff_coverage
                .as_ref()
                .map(|c| c.is_passed())
                .unwrap_or(true);

        let verdict = if is_blocked {
            VerdictStatus::Blocked
        } else if checks_all_passed {
            VerdictStatus::Success
        } else {
            VerdictStatus::Failed
        };

        // 9. Build Receipt
        let receipt = ExecutionReceipt {
            schema: DEFAULT_RECEIPT_SCHEMA.to_string(),
            task_id: config.task_id.clone(),
            workspace: workspace.clone(),
            verdict,
            checks: CheckSummary {
                compilation,
                tests,
                linter,
                scope_guard,
                diff_coverage,
            },
            artifacts: ArtifactSummary {
                git_patch_sha256,
                receipt_timestamp: Some(chrono::Utc::now().to_rfc3339()),
            },
        };

        // 9. Validate Receipt invariants
        receipt.validate()?;

        Ok(receipt)
    }
}

impl Default for WorkspaceVerifier {
    fn default() -> Self {
        Self::new()
    }
}

/// Computes the Git diff SHA-256 fingerprint of the repository relative to HEAD.
async fn compute_git_diff_sha256(workspace: &Path) -> Option<String> {
    let diff_output = Command::new("git")
        .args(["diff", "HEAD"])
        .current_dir(workspace)
        .output()
        .await;

    match diff_output {
        Ok(out) if out.status.success() => Some(ExecutionReceipt::compute_sha256(&out.stdout)),
        _ => {
            // Fallback for unborn HEAD (e.g. freshly initialized repository)
            let fallback = Command::new("git")
                .args(["diff"])
                .current_dir(workspace)
                .output()
                .await;
            match fallback {
                Ok(out) if out.status.success() => {
                    Some(ExecutionReceipt::compute_sha256(&out.stdout))
                }
                _ => None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use tempfile::TempDir;

    struct MockRunner {
        name: &'static str,
        compilation: CompilationCheck,
        tests: TestCheck,
        linter: LinterCheck,
    }

    #[async_trait]
    impl LanguageRunner for MockRunner {
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

    fn passing_runner() -> MockRunner {
        MockRunner {
            name: "mock",
            compilation: CompilationCheck {
                status: StepStatus::Passed,
                exit_code: 0,
                message: None,
            },
            tests: TestCheck {
                status: StepStatus::Passed,
                exit_code: Some(0),
                total: 5,
                failed: 0,
                duration_ms: Some(50),
                message: None,
                flaky_quarantined: None,
                retries: None,
            },
            linter: LinterCheck {
                status: StepStatus::Passed,
                warnings: 0,
                exit_code: Some(0),
                message: None,
            },
        }
    }

    fn setup_temp_git_repo() -> TempDir {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        let run_git = |args: &[&str]| {
            let output = std::process::Command::new("git")
                .args(args)
                .current_dir(root)
                .output()
                .expect("git execution");
            assert!(output.status.success());
        };

        run_git(&["init"]);
        run_git(&["config", "user.name", "Tester"]);
        run_git(&["config", "user.email", "tester@example.com"]);
        std::fs::write(root.join("README.md"), "# Test Repo").unwrap();
        run_git(&["add", "README.md"]);
        run_git(&["commit", "-m", "init"]);
        temp
    }

    #[test]
    fn test_verifier_config_builder() {
        let config = VerifierConfig::new("/tmp/test", "task-42")
            .whitelist(vec!["src/".to_string()])
            .override_runner("cargo".to_string())
            .auto_detect(false);

        assert_eq!(config.workspace, PathBuf::from("/tmp/test"));
        assert_eq!(config.task_id, "task-42");
        assert_eq!(config.whitelist, Some(vec!["src/".to_string()]));
        assert_eq!(config.override_runner.as_deref(), Some("cargo"));
        assert!(!config.auto_detect);

        let def = VerifierConfig::default();
        assert_eq!(def.workspace, PathBuf::from("."));
        assert_eq!(def.task_id, "task_ad_hoc");
        assert!(def.auto_detect);
    }

    #[tokio::test]
    async fn test_verify_all_passed_success() {
        let repo = setup_temp_git_repo();
        let mut registry = RunnerRegistry::empty();
        registry.register(Box::new(passing_runner()));

        let verifier = WorkspaceVerifier::with_registry(Arc::new(registry));
        let config = VerifierConfig::new(repo.path(), "task-ok");

        let receipt = verifier.verify(&config).await.expect("verify succeeds");
        assert_eq!(receipt.verdict, VerdictStatus::Success);
        assert!(receipt.is_passing());
        assert!(receipt.checks.compilation.is_passed());
        assert!(receipt.checks.tests.is_passed());
        assert!(receipt.checks.linter.is_passed());
        assert!(receipt.checks.scope_guard.is_passed());
        assert!(receipt.artifacts.git_patch_sha256.is_some());
        assert!(receipt.artifacts.receipt_timestamp.is_some());
    }

    #[tokio::test]
    async fn test_verify_compilation_failure() {
        let repo = setup_temp_git_repo();
        let mut runner = passing_runner();
        runner.compilation = CompilationCheck {
            status: StepStatus::Failed,
            exit_code: 1,
            message: Some("syntax error".to_string()),
        };

        let mut registry = RunnerRegistry::empty();
        registry.register(Box::new(runner));

        let verifier = WorkspaceVerifier::with_registry(Arc::new(registry));
        let config = VerifierConfig::new(repo.path(), "task-comp-fail");

        let receipt = verifier.verify(&config).await.expect("verify succeeds");
        assert_eq!(receipt.verdict, VerdictStatus::Failed);
        assert!(!receipt.is_passing());
        assert_eq!(receipt.checks.compilation.status, StepStatus::Failed);
    }

    #[tokio::test]
    async fn test_verify_tests_failure() {
        let repo = setup_temp_git_repo();
        let mut runner = passing_runner();
        runner.tests = TestCheck {
            status: StepStatus::Failed,
            exit_code: Some(101),
            total: 10,
            failed: 2,
            duration_ms: Some(100),
            message: Some("2 tests failed".to_string()),
            flaky_quarantined: None,
            retries: None,
        };

        let mut registry = RunnerRegistry::empty();
        registry.register(Box::new(runner));

        let verifier = WorkspaceVerifier::with_registry(Arc::new(registry));
        let config = VerifierConfig::new(repo.path(), "task-test-fail");

        let receipt = verifier.verify(&config).await.expect("verify succeeds");
        assert_eq!(receipt.verdict, VerdictStatus::Failed);
        assert!(!receipt.is_passing());
        assert_eq!(receipt.checks.tests.failed, 2);
    }

    #[tokio::test]
    async fn test_verify_linter_warnings_failure() {
        let repo = setup_temp_git_repo();
        let mut runner = passing_runner();
        runner.linter = LinterCheck {
            status: StepStatus::Failed,
            warnings: 3,
            exit_code: Some(1),
            message: Some("clippy warnings detected".to_string()),
        };

        let mut registry = RunnerRegistry::empty();
        registry.register(Box::new(runner));

        let verifier = WorkspaceVerifier::with_registry(Arc::new(registry));
        let config = VerifierConfig::new(repo.path(), "task-lint-fail");

        let receipt = verifier.verify(&config).await.expect("verify succeeds");
        assert_eq!(receipt.verdict, VerdictStatus::Failed);
        assert!(!receipt.is_passing());
        assert_eq!(receipt.checks.linter.warnings, 3);
    }

    #[tokio::test]
    async fn test_verify_scope_guard_violation() {
        let repo = setup_temp_git_repo();
        // Modify file outside declared whitelist
        std::fs::write(repo.path().join("unauthorized.env"), "SECRET=true").unwrap();

        let mut registry = RunnerRegistry::empty();
        registry.register(Box::new(passing_runner()));

        let verifier = WorkspaceVerifier::with_registry(Arc::new(registry));
        let config =
            VerifierConfig::new(repo.path(), "task-scope-fail").whitelist(vec!["src/".to_string()]);

        let receipt = verifier.verify(&config).await.expect("verify succeeds");
        assert_eq!(receipt.verdict, VerdictStatus::Failed);
        assert!(!receipt.is_passing());
        assert_eq!(receipt.checks.scope_guard.status, StepStatus::Failed);
        assert!(receipt
            .checks
            .scope_guard
            .unauthorized_files
            .contains(&"unauthorized.env".to_string()));
    }

    #[tokio::test]
    async fn test_verify_no_runner_detected_blocked() {
        let repo = setup_temp_git_repo();
        let registry = RunnerRegistry::empty();

        let verifier = WorkspaceVerifier::with_registry(Arc::new(registry));
        let config = VerifierConfig::new(repo.path(), "task-blocked");

        let receipt = verifier.verify(&config).await.expect("verify succeeds");
        assert_eq!(receipt.verdict, VerdictStatus::Blocked);
        assert!(!receipt.is_passing());
        assert_eq!(receipt.checks.compilation.status, StepStatus::Skipped);
        assert_eq!(receipt.checks.tests.status, StepStatus::Skipped);
        assert_eq!(receipt.checks.linter.status, StepStatus::Skipped);
    }

    #[tokio::test]
    async fn test_verify_override_runner_not_found_error() {
        let repo = setup_temp_git_repo();
        let registry = RunnerRegistry::empty();

        let verifier = WorkspaceVerifier::with_registry(Arc::new(registry));
        let config = VerifierConfig::new(repo.path(), "task-not-found")
            .override_runner("non_existent_runner".to_string());

        let err = verifier.verify(&config).await.expect_err("should error");
        match err {
            VerifierError::RunnerNotFound(msg) => {
                assert!(msg.contains("non_existent_runner"));
            }
            other => panic!("Unexpected error variant: {other:?}"),
        }
    }
}
