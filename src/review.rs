use crate::error::VerifierError;
use crate::receipt::{ExecutionReceipt, StepStatus, VerdictStatus};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::process::Command;

/// ReviewPack payload aggregating execution receipt, changed files, diff statistics,
/// and a concise Markdown executive summary for code review workflows.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ReviewPack {
    pub task_id: String,
    pub workspace: PathBuf,
    pub verdict: VerdictStatus,
    pub receipt: ExecutionReceipt,
    pub changed_files: Vec<String>,
    pub diff_stat: String,
    pub markdown_summary: String,
}

impl ReviewPack {
    /// Generates a `ReviewPack` from an `ExecutionReceipt`, extracting changed files,
    /// diff statistics against an optional base branch or HEAD, and formatting a concise Markdown summary.
    pub async fn generate(
        receipt: ExecutionReceipt,
        base_branch: Option<&str>,
    ) -> Result<Self, VerifierError> {
        let mut changed_files = receipt.checks.scope_guard.modified_files.clone();
        changed_files.sort();
        changed_files.dedup();

        let diff_stat = extract_diff_stat(&receipt.workspace, base_branch).await;
        let markdown_summary = format_markdown_summary(&receipt, &changed_files, &diff_stat);

        Ok(Self {
            task_id: receipt.task_id.clone(),
            workspace: receipt.workspace.clone(),
            verdict: receipt.verdict,
            receipt,
            changed_files,
            diff_stat,
            markdown_summary,
        })
    }

    /// Alias for `generate` to construct `ReviewPack` from an `ExecutionReceipt`.
    pub async fn from_receipt(
        receipt: ExecutionReceipt,
        base_branch: Option<&str>,
    ) -> Result<Self, VerifierError> {
        Self::generate(receipt, base_branch).await
    }

    /// Constructs a `ReviewPack` directly from a receipt and precomputed diff statistics.
    pub fn from_receipt_with_diff(receipt: ExecutionReceipt, diff_stat: String) -> Self {
        let mut changed_files = receipt.checks.scope_guard.modified_files.clone();
        changed_files.sort();
        changed_files.dedup();

        let markdown_summary = format_markdown_summary(&receipt, &changed_files, &diff_stat);

        Self {
            task_id: receipt.task_id.clone(),
            workspace: receipt.workspace.clone(),
            verdict: receipt.verdict,
            receipt,
            changed_files,
            diff_stat,
            markdown_summary,
        }
    }
}

/// Extracts `git diff --stat` output from the workspace.
///
/// Precedence:
/// 1. `git diff --stat <base_branch>` if `base_branch` is specified.
/// 2. `git diff --stat HEAD`
/// 3. `git diff --stat` (fallback for unborn HEAD or fresh repos)
///
/// Returns `"No diff detected"` if no diff is available or on failure.
pub async fn extract_diff_stat(workspace: &Path, base_branch: Option<&str>) -> String {
    if let Some(base) = base_branch {
        let base = base.trim();
        if !base.is_empty() {
            if let Ok(output) = Command::new("git")
                .args(["diff", "--stat", base])
                .current_dir(workspace)
                .output()
                .await
            {
                if output.status.success() {
                    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !text.is_empty() {
                        return text;
                    }
                }
            }
        }
    }

    if let Ok(output) = Command::new("git")
        .args(["diff", "--stat", "HEAD"])
        .current_dir(workspace)
        .output()
        .await
    {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !text.is_empty() {
                return text;
            }
        }
    }

    if let Ok(output) = Command::new("git")
        .args(["diff", "--stat"])
        .current_dir(workspace)
        .output()
        .await
    {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !text.is_empty() {
                return text;
            }
        }
    }

    "No diff detected".to_string()
}

/// Formats a concise Markdown executive summary (strictly under 20 lines)
/// for injection into code-review workflows and tuicr notes.
pub fn format_markdown_summary(
    receipt: &ExecutionReceipt,
    changed_files: &[String],
    diff_stat: &str,
) -> String {
    let verdict_str = match receipt.verdict {
        VerdictStatus::Success => "✅ Success",
        VerdictStatus::Failed => "❌ Failed",
        VerdictStatus::Blocked => "⚠️ Blocked",
    };

    let comp = match receipt.checks.compilation.status {
        StepStatus::Passed => "passed".to_string(),
        StepStatus::Failed => format!("failed (exit: {})", receipt.checks.compilation.exit_code),
        StepStatus::Skipped => "skipped".to_string(),
    };

    let tests = match receipt.checks.tests.status {
        StepStatus::Passed => {
            let passed_count = receipt
                .checks
                .tests
                .total
                .saturating_sub(receipt.checks.tests.failed);
            format!(
                "passed ({}/{} passed)",
                passed_count, receipt.checks.tests.total
            )
        }
        StepStatus::Failed => {
            format!(
                "failed ({}/{} failed)",
                receipt.checks.tests.failed, receipt.checks.tests.total
            )
        }
        StepStatus::Skipped => "skipped".to_string(),
    };

    let lint = match receipt.checks.linter.status {
        StepStatus::Passed => "passed (0 warnings)".to_string(),
        StepStatus::Failed => format!("failed ({} warnings)", receipt.checks.linter.warnings),
        StepStatus::Skipped => "skipped".to_string(),
    };

    let scope = match receipt.checks.scope_guard.status {
        StepStatus::Passed => "passed".to_string(),
        StepStatus::Failed => format!(
            "failed ({} unauthorized)",
            receipt.checks.scope_guard.unauthorized_files.len()
        ),
        StepStatus::Skipped => "skipped".to_string(),
    };

    let patch_sha = receipt
        .artifacts
        .git_patch_sha256
        .as_deref()
        .unwrap_or("none");

    let changed_files_preview = if changed_files.is_empty() {
        "none".to_string()
    } else if changed_files.len() <= 4 {
        changed_files
            .iter()
            .map(|f| format!("`{f}`"))
            .collect::<Vec<_>>()
            .join(", ")
    } else {
        let preview: Vec<String> = changed_files
            .iter()
            .take(4)
            .map(|f| format!("`{f}`"))
            .collect();
        format!(
            "{}, (+{} more)",
            preview.join(", "),
            changed_files.len() - 4
        )
    };

    let diff_trimmed = diff_stat.trim();
    let formatted_diff = if diff_trimmed.is_empty() {
        "No diff detected".to_string()
    } else {
        let diff_lines: Vec<&str> = diff_trimmed.lines().collect();
        if diff_lines.len() <= 8 {
            diff_trimmed.to_string()
        } else {
            let head = &diff_lines[..5];
            let tail = diff_lines.last().copied().unwrap_or("");
            let remaining = diff_lines.len() - 6;
            format!(
                "{}\n ... ({} more lines) ...\n{}",
                head.join("\n"),
                remaining,
                tail
            )
        }
    };

    format!(
        "### ⚖️ Review Pack: {}\n\
        - **Verdict**: {}\n\
        - **Checks**: Compilation: {} | Tests: {} | Linter: {} | Scope Guard: {}\n\
        - **Git Patch SHA-256**: `{}`\n\
        - **Changed Files ({})**: {}\n\n\
        ```text\n\
        {}\n\
        ```",
        receipt.task_id,
        verdict_str,
        comp,
        tests,
        lint,
        scope,
        patch_sha,
        changed_files.len(),
        changed_files_preview,
        formatted_diff
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::receipt::*;
    use tempfile::TempDir;

    fn sample_receipt() -> ExecutionReceipt {
        ExecutionReceipt {
            schema: DEFAULT_RECEIPT_SCHEMA.to_string(),
            task_id: "sample-task-001".to_string(),
            workspace: PathBuf::from("/workspace"),
            verdict: VerdictStatus::Success,
            checks: CheckSummary {
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
                scope_guard: ScopeGuardCheck {
                    status: StepStatus::Passed,
                    modified_files: vec![
                        "src/lib.rs".to_string(),
                        "src/main.rs".to_string(),
                        "src/lib.rs".to_string(), // duplicate to test dedup
                    ],
                    unauthorized_files: Vec::new(),
                    message: None,
                },
            },
            artifacts: ArtifactSummary {
                git_patch_sha256: Some(
                    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
                ),
                receipt_timestamp: Some("2026-10-12T16:00:00Z".to_string()),
            },
        }
    }

    #[test]
    fn test_review_pack_serde_roundtrip() {
        let receipt = sample_receipt();
        let pack = ReviewPack::from_receipt_with_diff(
            receipt.clone(),
            " src/lib.rs | 2 +-\n 1 file changed, 1 insertion(+), 1 deletion(-)".to_string(),
        );

        let json = serde_json::to_string_pretty(&pack).expect("serialize ReviewPack");
        let deserialized: ReviewPack = serde_json::from_str(&json).expect("deserialize ReviewPack");

        assert_eq!(pack, deserialized);
        assert_eq!(deserialized.task_id, "sample-task-001");
        assert_eq!(deserialized.verdict, VerdictStatus::Success);
        assert_eq!(
            deserialized.changed_files,
            vec!["src/lib.rs", "src/main.rs"]
        );
    }

    #[test]
    fn test_markdown_summary_under_20_lines_always() {
        let mut receipt = sample_receipt();

        // 1. Clean / empty diff
        let summary_clean = format_markdown_summary(&receipt, &[], "No diff detected");
        assert!(summary_clean.lines().count() < 20);
        assert!(summary_clean.contains("### ⚖️ Review Pack: sample-task-001"));
        assert!(summary_clean.contains("✅ Success"));
        assert!(summary_clean.contains("No diff detected"));

        // 2. Normal diff
        let normal_diff = " src/lib.rs  | 5 +++++\n src/main.rs | 2 +-\n 2 files changed, 6 insertions(+), 1 deletion(-)";
        let summary_normal = format_markdown_summary(
            &receipt,
            &["src/lib.rs".to_string(), "src/main.rs".to_string()],
            normal_diff,
        );
        assert!(summary_normal.lines().count() < 20);

        // 3. Huge diff with 30 files and 50 changed files list
        let mut huge_diff_lines = Vec::new();
        for i in 0..30 {
            huge_diff_lines.push(format!(" file_{i}.rs | 10 ++++++++++"));
        }
        huge_diff_lines.push(" 30 files changed, 300 insertions(+)".to_string());
        let huge_diff = huge_diff_lines.join("\n");

        let mut many_files = Vec::new();
        for i in 0..50 {
            many_files.push(format!("file_{i}.rs"));
        }

        receipt.checks.scope_guard.modified_files = many_files.clone();
        let summary_huge = format_markdown_summary(&receipt, &many_files, &huge_diff);

        let line_count = summary_huge.lines().count();
        assert!(
            line_count < 20,
            "Markdown summary must be strictly under 20 lines, got {line_count}"
        );
        assert!(summary_huge.contains("... (25 more lines) ..."));
        assert!(summary_huge.contains("(+46 more)"));
    }

    #[test]
    fn test_changed_files_sorting_and_deduplication() {
        let mut receipt = sample_receipt();
        receipt.checks.scope_guard.modified_files = vec![
            "z_file.rs".to_string(),
            "a_file.rs".to_string(),
            "m_file.rs".to_string(),
            "a_file.rs".to_string(),
            "z_file.rs".to_string(),
        ];

        let pack = ReviewPack::from_receipt_with_diff(receipt, "diff".to_string());
        assert_eq!(
            pack.changed_files,
            vec!["a_file.rs", "m_file.rs", "z_file.rs"]
        );
    }

    #[test]
    fn test_extract_diff_stat_non_git_dir() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let temp = TempDir::new().unwrap();
            let stat = extract_diff_stat(temp.path(), None).await;
            assert_eq!(stat, "No diff detected");
        });
    }

    #[test]
    fn test_review_pack_generate_in_git_repo() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        let run_git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(root)
                .output()
                .expect("git exec");
            assert!(out.status.success());
        };

        run_git(&["init"]);
        run_git(&["config", "user.name", "ReviewTester"]);
        run_git(&["config", "user.email", "tester@review.com"]);

        std::fs::write(root.join("hello.txt"), "hello world\n").unwrap();
        run_git(&["add", "hello.txt"]);
        run_git(&["commit", "-m", "init"]);

        // Modify tracked file
        std::fs::write(root.join("hello.txt"), "hello world modified\n").unwrap();

        let mut receipt = sample_receipt();
        receipt.workspace = root.to_path_buf();
        receipt.checks.scope_guard.modified_files = vec!["hello.txt".to_string()];

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let pack = ReviewPack::generate(receipt.clone(), None)
                .await
                .expect("generate review pack");

            assert_eq!(pack.task_id, "sample-task-001");
            assert_eq!(pack.workspace, root);
            assert_eq!(pack.changed_files, vec!["hello.txt"]);
            assert!(pack.diff_stat.contains("hello.txt"));
            assert!(pack.markdown_summary.lines().count() < 20);
            assert!(pack
                .markdown_summary
                .contains("### ⚖️ Review Pack: sample-task-001"));

            // Test alias from_receipt
            let pack_alias = ReviewPack::from_receipt(receipt, None)
                .await
                .expect("from_receipt alias");
            assert_eq!(pack, pack_alias);
        });
    }
}
