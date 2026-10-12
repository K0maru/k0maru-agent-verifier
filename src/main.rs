use clap::{Args, Parser, Subcommand};
use k0maru_verifier::receipt::{ExecutionReceipt, StepStatus, VerdictStatus};
use k0maru_verifier::verifier::{VerifierConfig, WorkspaceVerifier};
use serde::Deserialize;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser, Debug)]
#[command(
    name = "k0maru-verifier",
    about = "Deterministic test oracle and execution receipt signer for K0maru-Harness",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    #[arg(long, default_value = ".")]
    pub workspace: PathBuf,

    #[arg(long)]
    pub task_id: Option<String>,

    #[arg(long, value_delimiter = ',')]
    pub whitelist: Option<Vec<String>>,

    #[arg(long)]
    pub out: Option<PathBuf>,

    #[arg(long)]
    pub json: bool,

    #[arg(long)]
    pub runner: Option<String>,

    #[arg(long)]
    pub spec: Option<PathBuf>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum Commands {
    /// Verify workspace quality gates and produce an execution receipt.
    Verify(VerifyArgs),
}

#[derive(Args, Debug, Clone, Default)]
pub struct VerifyArgs {
    #[arg(long, default_value = ".")]
    pub workspace: PathBuf,

    #[arg(long)]
    pub task_id: Option<String>,

    #[arg(long, value_delimiter = ',')]
    pub whitelist: Option<Vec<String>>,

    #[arg(long)]
    pub out: Option<PathBuf>,

    #[arg(long)]
    pub json: bool,

    #[arg(long)]
    pub runner: Option<String>,

    #[arg(long)]
    pub spec: Option<PathBuf>,
}

impl Cli {
    /// Flattens either the `verify` subcommand or root-level flags into unified `VerifyArgs`.
    pub fn resolve(self) -> VerifyArgs {
        match self.command {
            Some(Commands::Verify(args)) => args,
            None => VerifyArgs {
                workspace: self.workspace,
                task_id: self.task_id,
                whitelist: self.whitelist,
                out: self.out,
                json: self.json,
                runner: self.runner,
                spec: self.spec,
            },
        }
    }
}

/// Optional JSON spec defining task parameters.
#[derive(Debug, Deserialize, Default)]
pub struct TaskSpec {
    pub task_id: Option<String>,
    pub workspace: Option<String>,
    pub whitelist: Option<Vec<String>>,
    pub allowed_files: Option<Vec<String>>,
    pub runner: Option<String>,
}

/// Merges command-line arguments with an optional specification file.
pub fn merge_args_with_spec(args: &VerifyArgs) -> Result<VerifierConfig, i32> {
    let spec = if let Some(ref spec_path) = args.spec {
        let content = std::fs::read_to_string(spec_path).map_err(|e| {
            eprintln!("❌ Failed to read spec file '{}': {e}", spec_path.display());
            2
        })?;
        let parsed: TaskSpec = serde_json::from_str(&content).map_err(|e| {
            eprintln!(
                "❌ Failed to parse spec file JSON '{}': {e}",
                spec_path.display()
            );
            2
        })?;
        Some(parsed)
    } else {
        None
    };

    let workspace =
        if args.workspace != std::path::Path::new(".") && !args.workspace.as_os_str().is_empty() {
            args.workspace.clone()
        } else if let Some(ref s) = spec {
            s.workspace.as_ref().map(PathBuf::from).unwrap_or_else(|| {
                if args.workspace.as_os_str().is_empty() {
                    PathBuf::from(".")
                } else {
                    args.workspace.clone()
                }
            })
        } else if args.workspace.as_os_str().is_empty() {
            PathBuf::from(".")
        } else {
            args.workspace.clone()
        };

    let task_id = args
        .task_id
        .clone()
        .or_else(|| spec.as_ref().and_then(|s| s.task_id.clone()))
        .unwrap_or_else(|| "task_ad_hoc".to_string());

    let whitelist = args.whitelist.clone().or_else(|| {
        spec.as_ref()
            .and_then(|s| s.whitelist.clone().or_else(|| s.allowed_files.clone()))
    });

    let override_runner = args
        .runner
        .clone()
        .or_else(|| spec.as_ref().and_then(|s| s.runner.clone()));

    Ok(VerifierConfig::new(workspace, task_id)
        .whitelist(whitelist)
        .override_runner(override_runner)
        .auto_detect(true))
}

/// Formats a human-readable ANSI colored summary of the verification receipt.
pub fn format_pretty_summary(receipt: &ExecutionReceipt) -> String {
    let verdict_badge = match receipt.verdict {
        VerdictStatus::Success => "\x1b[1;32m✅ SUCCESS\x1b[0m",
        VerdictStatus::Failed => "\x1b[1;31m❌ FAILED\x1b[0m",
        VerdictStatus::Blocked => "\x1b[1;33m⚠️  BLOCKED\x1b[0m",
    };

    let comp_icon = match receipt.checks.compilation.status {
        StepStatus::Passed => "✅",
        StepStatus::Failed => "❌",
        StepStatus::Skipped => "⏭️ ",
    };
    let test_icon = match receipt.checks.tests.status {
        StepStatus::Passed => "✅",
        StepStatus::Failed => "❌",
        StepStatus::Skipped => "⏭️ ",
    };
    let lint_icon = match receipt.checks.linter.status {
        StepStatus::Passed => "✅",
        StepStatus::Failed => "❌",
        StepStatus::Skipped => "⏭️ ",
    };
    let scope_icon = match receipt.checks.scope_guard.status {
        StepStatus::Passed => "✅",
        StepStatus::Failed => "❌",
        StepStatus::Skipped => "⏭️ ",
    };

    let comp_detail = format!(
        "{} (exit code: {})",
        receipt.checks.compilation.status, receipt.checks.compilation.exit_code
    );
    let test_detail = format!(
        "{} ({}/{} failed, duration: {}ms)",
        receipt.checks.tests.status,
        receipt.checks.tests.failed,
        receipt.checks.tests.total,
        receipt.checks.tests.duration_ms.unwrap_or(0)
    );
    let lint_detail = format!(
        "{} (warnings: {})",
        receipt.checks.linter.status, receipt.checks.linter.warnings
    );
    let scope_detail = format!(
        "{} (modified: {}, unauthorized: {})",
        receipt.checks.scope_guard.status,
        receipt.checks.scope_guard.modified_files.len(),
        receipt.checks.scope_guard.unauthorized_files.len()
    );

    let patch_sha = receipt
        .artifacts
        .git_patch_sha256
        .as_deref()
        .unwrap_or("<none>");
    let timestamp = receipt
        .artifacts
        .receipt_timestamp
        .as_deref()
        .unwrap_or("<none>");

    let mut out = format!(
        r#"
⚖️  [K0maru-Verifier] Deterministic Quality Gate
══════════════════════════════════════════════════════════════════
  Task ID:      {}
  Workspace:    {}
  Verdict:      {}
──────────────────────────────────────────────────────────────────
  Checks:
    {} Compilation:  {}
    {} Tests:        {}
    {} Linter:       {}
    {} Scope Guard:  {}
──────────────────────────────────────────────────────────────────
  Artifacts:
    📜 Git SHA-256:  {}
    🕒 Timestamp:    {}
══════════════════════════════════════════════════════════════════
"#,
        receipt.task_id,
        receipt.workspace.display(),
        verdict_badge,
        comp_icon,
        comp_detail,
        test_icon,
        test_detail,
        lint_icon,
        lint_detail,
        scope_icon,
        scope_detail,
        patch_sha,
        timestamp,
    );

    if !receipt.checks.scope_guard.unauthorized_files.is_empty() {
        out.push_str("\n🚨 Unauthorized modified files detected:\n");
        for f in &receipt.checks.scope_guard.unauthorized_files {
            out.push_str(&format!("  - {f}\n"));
        }
    }

    out
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let args = cli.resolve();

    let config = match merge_args_with_spec(&args) {
        Ok(c) => c,
        Err(code) => return ExitCode::from(code as u8),
    };

    let verifier = WorkspaceVerifier::new();
    let receipt = match verifier.verify(&config).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("❌ Fatal verification error: {e}");
            return ExitCode::from(2);
        }
    };

    let json_receipt = match serde_json::to_string_pretty(&receipt) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("❌ Failed to serialize execution receipt: {e}");
            return ExitCode::from(2);
        }
    };

    if args.json {
        println!("{json_receipt}");
    } else {
        println!("{}", format_pretty_summary(&receipt));
    }

    if let Some(ref out_path) = args.out {
        if let Err(e) = std::fs::write(out_path, &json_receipt) {
            eprintln!(
                "❌ Failed to write receipt to '{}': {e}",
                out_path.display()
            );
            return ExitCode::from(2);
        }
        if !args.json {
            println!("📜 Receipt written to {}", out_path.display());
        }
    }

    let code: u8 = match receipt.verdict {
        VerdictStatus::Success => 0,
        VerdictStatus::Failed => 1,
        VerdictStatus::Blocked => 2,
    };

    ExitCode::from(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use tempfile::NamedTempFile;

    #[test]
    fn test_cli_subcommand_parsing() {
        let args = vec![
            "k0maru-verifier",
            "verify",
            "--workspace",
            "/tmp/demo",
            "--task-id",
            "t-1",
        ];
        let cli = Cli::try_parse_from(args).expect("parse subcommand");
        let resolved = cli.resolve();
        assert_eq!(resolved.workspace, PathBuf::from("/tmp/demo"));
        assert_eq!(resolved.task_id.as_deref(), Some("t-1"));
    }

    #[test]
    fn test_cli_root_flags_parsing() {
        let args = vec![
            "k0maru-verifier",
            "--workspace",
            "/tmp/demo",
            "--task-id",
            "t-2",
            "--whitelist",
            "src/,tests/",
            "--json",
        ];
        let cli = Cli::try_parse_from(args).expect("parse root flags");
        let resolved = cli.resolve();
        assert_eq!(resolved.workspace, PathBuf::from("/tmp/demo"));
        assert_eq!(resolved.task_id.as_deref(), Some("t-2"));
        assert_eq!(
            resolved.whitelist,
            Some(vec!["src/".to_string(), "tests/".to_string()])
        );
        assert!(resolved.json);
    }

    #[test]
    fn test_merge_args_with_spec_file() {
        let temp = NamedTempFile::new().unwrap();
        let spec_json = r#"
{
  "task_id": "spec-task-99",
  "workspace": "/tmp/from-spec",
  "whitelist": ["src/lib.rs", "Cargo.toml"],
  "runner": "cargo"
}
"#;
        std::fs::write(temp.path(), spec_json).unwrap();

        let args = VerifyArgs {
            spec: Some(temp.path().to_path_buf()),
            ..Default::default()
        };

        let config = merge_args_with_spec(&args).expect("merge succeeds");
        assert_eq!(config.task_id, "spec-task-99");
        assert_eq!(config.workspace, PathBuf::from("/tmp/from-spec"));
        assert_eq!(
            config.whitelist,
            Some(vec!["src/lib.rs".to_string(), "Cargo.toml".to_string()])
        );
        assert_eq!(config.override_runner.as_deref(), Some("cargo"));
    }

    #[test]
    fn test_merge_args_cli_overrides_spec() {
        let temp = NamedTempFile::new().unwrap();
        let spec_json = r#"
{
  "task_id": "spec-task-1",
  "runner": "pytest"
}
"#;
        std::fs::write(temp.path(), spec_json).unwrap();

        let args = VerifyArgs {
            task_id: Some("cli-override-task".to_string()),
            runner: Some("cargo".to_string()),
            spec: Some(temp.path().to_path_buf()),
            ..Default::default()
        };

        let config = merge_args_with_spec(&args).expect("merge succeeds");
        assert_eq!(config.task_id, "cli-override-task");
        assert_eq!(config.override_runner.as_deref(), Some("cargo"));
    }

    #[test]
    fn test_format_pretty_summary_output() {
        use k0maru_verifier::receipt::*;

        let receipt = ExecutionReceipt {
            schema: DEFAULT_RECEIPT_SCHEMA.to_string(),
            task_id: "test-task-123".to_string(),
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
                    duration_ms: Some(120),
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
                    modified_files: vec!["src/main.rs".to_string()],
                    unauthorized_files: Vec::new(),
                },
            },
            artifacts: ArtifactSummary {
                git_patch_sha256: Some("abcdef1234567890".to_string()),
                receipt_timestamp: Some("2026-10-12T15:00:00Z".to_string()),
            },
        };

        let summary = format_pretty_summary(&receipt);
        assert!(summary.contains("test-task-123"));
        assert!(summary.contains("/workspace"));
        assert!(summary.contains("SUCCESS"));
        assert!(summary.contains("Compilation:  passed (exit code: 0)"));
        assert!(summary.contains("Tests:        passed (0/10 failed, duration: 120ms)"));
        assert!(summary.contains("Linter:       passed (warnings: 0)"));
        assert!(summary.contains("Scope Guard:  passed (modified: 1, unauthorized: 0)"));
        assert!(summary.contains("abcdef1234567890"));
    }
}
