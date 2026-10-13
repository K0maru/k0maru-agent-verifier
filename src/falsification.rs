//! Popperian Physical Falsification Engine (`--red-check`).
//!
//! Inspired by Popperian epistemology and autonomous verification principles (Magpie & AgentSWE),
//! this module defeats "fake green" agent-generated tests.
//!
//! # Principle:
//! A valid test for a bugfix or new feature MUST be capable of failing on the unmodified
//! baseline code. If new tests pass even when production changes are reverted, the tests are
//! unfalsifiable (tautological, mocked out, or testing nothing).
//!
//! # Execution Workflow:
//! 1. Detect modified/added `test_files` and `prod_files` relative to the baseline.
//! 2. If no tests or no production files were modified, skip falsification gracefully.
//! 3. Safely back up current `prod_files` in-memory with RAII restoration guard.
//! 4. Temporarily revert `prod_files` to baseline (`git show <base>:<file>`).
//! 5. Run test runner against baseline code:
//!    - Tests FAIL -> **Falsification Confirmed** (`StepStatus::Passed`).
//!    - Tests PASS -> **Fake Green Detected** (`StepStatus::Failed`).
//! 6. Restore `prod_files` to current working tree state.

use crate::coverage::is_source_code_file;
use crate::error::VerifierError;
use crate::guard::{collect_modified_files, is_test_file, normalize_path};
use crate::receipt::{RedCheckResult, StepStatus};
use crate::runner::LanguageRunner;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use tokio::process::Command;

/// Checks whether a path corresponds to build artifacts, VCS, or dependency caches.
pub fn is_ignored_or_build_artifact(path: &str) -> bool {
    let norm = normalize_path(path);
    let build_dirs = [
        "target/",
        "node_modules/",
        ".git/",
        "dist/",
        "build/",
        "__pycache__/",
        ".pytest_cache/",
        ".coverage",
        ".mypy_cache/",
        ".venv/",
        "venv/",
    ];
    build_dirs
        .iter()
        .any(|d| norm.starts_with(d) || norm == d.trim_end_matches('/'))
}

/// Checks whether a modified file should be treated as production code for falsification.
pub fn is_falsification_prod_file(path: &str) -> bool {
    let norm = normalize_path(path);
    if is_ignored_or_build_artifact(&norm) || is_test_file(&norm) {
        return false;
    }

    if is_source_code_file(&norm) {
        return true;
    }

    // Include project manifests that affect build behavior
    let manifests = [
        "Cargo.toml",
        "package.json",
        "pyproject.toml",
        "setup.py",
        "go.mod",
        "pom.xml",
        "build.gradle",
    ];

    manifests
        .iter()
        .any(|m| norm == *m || norm.ends_with(&format!("/{m}")))
}

/// RAII Guard that captures original file bytes in memory and guarantees restoration upon drop.
pub struct FileBackupGuard {
    workspace: PathBuf,
    backups: BTreeMap<String, Option<Vec<u8>>>,
    active: bool,
}

impl FileBackupGuard {
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
            backups: BTreeMap::new(),
            active: true,
        }
    }

    /// Backs up the current on-disk state of a workspace-relative file path.
    pub fn backup_file(&mut self, rel_path: &str) -> Result<(), std::io::Error> {
        let norm = normalize_path(rel_path);
        let full_path = self.workspace.join(&norm);
        let content = if full_path.exists() {
            Some(std::fs::read(&full_path)?)
        } else {
            None
        };
        self.backups.insert(norm, content);
        Ok(())
    }

    /// Restores all backed up files to their original on-disk state.
    pub fn restore(&mut self) {
        if !self.active {
            return;
        }

        for (rel_path, maybe_bytes) in &self.backups {
            let full_path = self.workspace.join(rel_path);
            match maybe_bytes {
                Some(bytes) => {
                    if let Some(parent) = full_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let _ = std::fs::write(&full_path, bytes);
                }
                None => {
                    if full_path.exists() {
                        let _ = std::fs::remove_file(&full_path);
                    }
                }
            }
        }

        self.active = false;
    }
}

impl Drop for FileBackupGuard {
    fn drop(&mut self) {
        self.restore();
    }
}

/// Resolves the Git baseline reference to compare against.
pub async fn resolve_red_check_baseline(
    workspace: &Path,
    explicit_branch: Option<&str>,
) -> Result<String, VerifierError> {
    if let Some(b) = explicit_branch {
        let trimmed = b.trim();
        if !trimmed.is_empty() {
            let out = Command::new("git")
                .args(["rev-parse", "--verify", trimmed])
                .current_dir(workspace)
                .output()
                .await
                .map_err(|e| {
                    VerifierError::GitError(format!("Failed to verify branch '{trimmed}': {e}"))
                })?;
            if out.status.success() {
                return Ok(trimmed.to_string());
            } else {
                return Err(VerifierError::GitError(format!(
                    "Explicit base branch '{trimmed}' does not exist in git repository"
                )));
            }
        }
    }

    // If working tree has modified or untracked files, HEAD is the baseline
    let status_out = Command::new("git")
        .args(["status", "--porcelain", "-uall"])
        .current_dir(workspace)
        .output()
        .await;

    if let Ok(out) = status_out {
        let stdout = String::from_utf8_lossy(&out.stdout);
        if !stdout.trim().is_empty() {
            return Ok("HEAD".to_string());
        }
    }

    // If working tree is clean, check if HEAD~1 exists
    let head_prev = Command::new("git")
        .args(["rev-parse", "--verify", "HEAD~1"])
        .current_dir(workspace)
        .output()
        .await;

    if let Ok(out) = head_prev {
        if out.status.success() {
            return Ok("HEAD~1".to_string());
        }
    }

    Ok("HEAD".to_string())
}

/// Collects changed files between the workspace and the given baseline reference.
pub async fn collect_red_check_diff_files(
    workspace: &Path,
    baseline: &str,
) -> Result<Vec<String>, VerifierError> {
    let mut files = BTreeSet::new();

    // 1. Collect files from git diff --name-only <baseline>
    let diff_out = Command::new("git")
        .args(["diff", "--name-only", baseline])
        .current_dir(workspace)
        .output()
        .await
        .map_err(|e| {
            VerifierError::GitError(format!("Failed to run git diff against '{baseline}': {e}"))
        })?;

    if diff_out.status.success() {
        for line in String::from_utf8_lossy(&diff_out.stdout).lines() {
            let norm = normalize_path(line);
            if !norm.is_empty() {
                files.insert(norm);
            }
        }
    }

    // 2. Also incorporate uncommitted / untracked files
    if let Ok(dirty_files) = collect_modified_files(workspace).await {
        for f in dirty_files {
            files.insert(f);
        }
    }

    Ok(files.into_iter().collect())
}

/// Executes Popperian physical falsification on the workspace.
pub async fn run_red_check(
    workspace: &Path,
    runner: &dyn LanguageRunner,
    base_branch: Option<&str>,
) -> Result<RedCheckResult, VerifierError> {
    let baseline = resolve_red_check_baseline(workspace, base_branch).await?;
    let changed_files = collect_red_check_diff_files(workspace, &baseline).await?;

    let mut test_files = Vec::new();
    let mut prod_files = Vec::new();

    for f in changed_files {
        if is_ignored_or_build_artifact(&f) {
            continue;
        }
        if is_test_file(&f) {
            test_files.push(f);
        } else if is_falsification_prod_file(&f) {
            prod_files.push(f);
        }
    }

    test_files.sort();
    test_files.dedup();
    prod_files.sort();
    prod_files.dedup();

    if test_files.is_empty() {
        return Ok(RedCheckResult {
            status: StepStatus::Skipped,
            baseline_ref: Some(baseline),
            test_files: Vec::new(),
            prod_files,
            exit_code: None,
            message: Some(
                "No new or modified test files detected for Popperian falsification".to_string(),
            ),
        });
    }

    if prod_files.is_empty() {
        return Ok(RedCheckResult {
            status: StepStatus::Skipped,
            baseline_ref: Some(baseline),
            test_files,
            prod_files: Vec::new(),
            exit_code: None,
            message: Some("No production source files modified to falsify against".to_string()),
        });
    }

    // Safe RAII backup guard for all modified production files
    let mut guard = FileBackupGuard::new(workspace);
    for file in &prod_files {
        guard.backup_file(file).map_err(VerifierError::IoError)?;
    }

    // Temporarily revert production files to baseline
    for file in &prod_files {
        let full_path = workspace.join(file);
        let git_show = Command::new("git")
            .args(["show", &format!("{baseline}:{file}")])
            .current_dir(workspace)
            .output()
            .await;

        match git_show {
            Ok(out) if out.status.success() => {
                if let Some(parent) = full_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                std::fs::write(&full_path, &out.stdout).map_err(VerifierError::IoError)?;
            }
            _ => {
                // File did not exist at baseline (newly added file), remove it temporarily
                if full_path.exists() {
                    let _ = std::fs::remove_file(&full_path);
                }
            }
        }
    }

    // Run test runner against baseline code with new tests
    let test_res = runner.test(workspace).await;

    // Explicitly restore original files before evaluating results
    guard.restore();

    match test_res {
        Ok(t) => {
            if !t.is_passed() {
                // Test failed on baseline -> Popperian falsification confirmed!
                let exit_code = t.exit_code.unwrap_or(1);
                Ok(RedCheckResult {
                    status: StepStatus::Passed,
                    baseline_ref: Some(baseline.clone()),
                    test_files,
                    prod_files,
                    exit_code: Some(exit_code),
                    message: Some(format!(
                        "Popperian falsification confirmed: tests correctly failed on unmodified baseline '{baseline}' (exit code: {exit_code}, failed tests: {})",
                        t.failed
                    )),
                })
            } else {
                // Test passed on baseline -> FAKE GREEN DETECTED!
                Ok(RedCheckResult {
                    status: StepStatus::Failed,
                    baseline_ref: Some(baseline.clone()),
                    test_files,
                    prod_files,
                    exit_code: t.exit_code,
                    message: Some(format!(
                        "Popperian falsification failed: tests passed against unmodified baseline '{baseline}' without production changes (fake green test detected)"
                    )),
                })
            }
        }
        Err(e) => {
            // Test runner returned an error on baseline (e.g. compilation error) -> count as failure on baseline
            Ok(RedCheckResult {
                status: StepStatus::Passed,
                baseline_ref: Some(baseline.clone()),
                test_files,
                prod_files,
                exit_code: Some(1),
                message: Some(format!(
                    "Popperian falsification confirmed: tests failed on unmodified baseline '{baseline}' with runner error ({e})"
                )),
            })
        }
    }
}
