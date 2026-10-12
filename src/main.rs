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
    long_about = "Deterministic test oracle and execution receipt signer for K0maru-Harness.\n\n\
                  Running without a subcommand executes verification directly:\n  \
                  k0maru-verifier [OPTIONS]\n\n\
                  The 'verify' subcommand is also supported:\n  \
                  k0maru-verifier verify [OPTIONS]",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    #[command(flatten)]
    pub root_args: VerifyArgs,
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum Commands {
    /// Verify workspace quality gates and produce an execution receipt (default action)
    #[command(alias = "run")]
    Verify(VerifyArgs),
    #[command(name = "review-pack")]
    /// Verify workspace quality gates and generate a ReviewPack for review integration
    ReviewPack(VerifyArgs),
    #[command(name = "baseline")]
    /// Verify workspace quality gates enforcing immutable test baseline protection
    Baseline(VerifyArgs),
    #[command(name = "diff")]
    /// Verify workspace quality gates with scope guard and diff inspection
    Diff(VerifyArgs),
}

#[derive(Args, Debug, Clone, Default, PartialEq, Eq)]
pub struct VerifyArgs {
    /// Target workspace directory to verify
    #[arg(long, default_value = ".")]
    pub workspace: PathBuf,

    /// Unique task identifier for correlation
    #[arg(long)]
    pub task_id: Option<String>,

    /// Allowed file paths or prefixes whitelist (comma-separated)
    #[arg(long, value_delimiter = ',')]
    pub whitelist: Option<Vec<String>>,

    /// File path to write the signed receipt JSON output
    #[arg(long)]
    pub out: Option<PathBuf>,

    /// Emit machine-readable execution receipt JSON to stdout
    #[arg(long)]
    pub json: bool,

    /// Explicitly override language runner (e.g. cargo, pytest, npm)
    #[arg(long)]
    pub runner: Option<String>,

    /// Path to a JSON specification file containing task parameters
    #[arg(long)]
    pub spec: Option<PathBuf>,

    /// Base branch for immutable test baseline comparison (e.g. main, dev)
    #[arg(long)]
    pub base_branch: Option<String>,

    /// Protect existing tests from line deletions against base branch
    #[arg(long)]
    pub immutable_tests: bool,

    /// Generate a ReviewPack executive summary and payload for review integration
    #[arg(long)]
    pub review_pack: bool,
}

impl VerifyArgs {
    /// Merges another `VerifyArgs` as a fallback for unset values.
    pub fn merge_fallback(mut self, fallback: VerifyArgs) -> Self {
        let is_default_ws =
            self.workspace.as_os_str().is_empty() || self.workspace == std::path::Path::new(".");
        let has_custom_fallback_ws = !fallback.workspace.as_os_str().is_empty()
            && fallback.workspace != std::path::Path::new(".");

        if is_default_ws && has_custom_fallback_ws {
            self.workspace = fallback.workspace;
        }
        if self.task_id.is_none() {
            self.task_id = fallback.task_id;
        }
        if self.whitelist.is_none() {
            self.whitelist = fallback.whitelist;
        }
        if self.out.is_none() {
            self.out = fallback.out;
        }
        self.json = self.json || fallback.json;
        if self.runner.is_none() {
            self.runner = fallback.runner;
        }
        if self.spec.is_none() {
            self.spec = fallback.spec;
        }
        if self.base_branch.is_none() {
            self.base_branch = fallback.base_branch;
        }
        self.immutable_tests = self.immutable_tests || fallback.immutable_tests;
        self.review_pack = self.review_pack || fallback.review_pack;
        self
    }
}

impl Cli {
    /// Resolves either the subcommand or root-level flags into unified `VerifyArgs`.
    pub fn resolve(self) -> VerifyArgs {
        match self.command {
            Some(Commands::Verify(sub_args)) => sub_args.merge_fallback(self.root_args),
            Some(Commands::ReviewPack(mut sub_args)) => {
                sub_args.review_pack = true;
                sub_args.merge_fallback(self.root_args)
            }
            Some(Commands::Baseline(mut sub_args)) => {
                sub_args.immutable_tests = true;
                sub_args.merge_fallback(self.root_args)
            }
            Some(Commands::Diff(sub_args)) => sub_args.merge_fallback(self.root_args),
            None => self.root_args,
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
    pub base_branch: Option<String>,
    pub immutable_tests: Option<bool>,
}

/// Merges command-line arguments with an optional specification file.
pub fn merge_args_with_spec(args: &VerifyArgs) -> Result<VerifierConfig, i32> {
    let spec = if let Some(ref spec_path) = args.spec {
        let content = std::fs::read_to_string(spec_path).map_err(|e| {
            eprintln!("❌ Failed to read spec file '{}': {e}", spec_path.display());
            k0maru_verifier::constants::EXIT_FATAL_ERROR
        })?;
        let parsed: TaskSpec = serde_json::from_str(&content).map_err(|e| {
            eprintln!(
                "❌ Failed to parse spec file JSON '{}': {e}",
                spec_path.display()
            );
            k0maru_verifier::constants::EXIT_FATAL_ERROR
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

    let base_branch = args
        .base_branch
        .clone()
        .or_else(|| spec.as_ref().and_then(|s| s.base_branch.clone()));

    let immutable_tests = args.immutable_tests
        || spec
            .as_ref()
            .and_then(|s| s.immutable_tests)
            .unwrap_or(false);

    Ok(VerifierConfig::new(workspace, task_id)
        .whitelist(whitelist)
        .override_runner(override_runner)
        .base_branch(base_branch)
        .immutable_tests(immutable_tests)
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

    if let Some(ref msg) = receipt.checks.scope_guard.message {
        out.push_str(&format!("\n⚠️  Scope Guard message: {msg}\n"));
    }

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
            return ExitCode::from(k0maru_verifier::constants::EXIT_FATAL_ERROR as u8);
        }
    };

    let json_receipt = match serde_json::to_string_pretty(&receipt) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("❌ Failed to serialize execution receipt: {e}");
            return ExitCode::from(k0maru_verifier::constants::EXIT_FATAL_ERROR as u8);
        }
    };

    let target_receipt_path = if let Some(ref out_path) = args.out {
        out_path.clone()
    } else {
        let receipts_dir = k0maru_verifier::receipt::resolve_receipts_dir(None);
        receipts_dir.join(format!("{}.json", receipt.task_id))
    };

    if let Some(parent) = target_receipt_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!(
                "❌ Failed to create receipt directory '{}': {e}",
                parent.display()
            );
            return ExitCode::from(k0maru_verifier::constants::EXIT_FATAL_ERROR as u8);
        }
    }

    if let Err(e) = std::fs::write(&target_receipt_path, &json_receipt) {
        eprintln!(
            "❌ Failed to write receipt to '{}': {e}",
            target_receipt_path.display()
        );
        return ExitCode::from(k0maru_verifier::constants::EXIT_FATAL_ERROR as u8);
    }

    if args.review_pack && receipt.verdict == VerdictStatus::Success {
        let review_pack = match k0maru_verifier::ReviewPack::generate(
            receipt.clone(),
            args.base_branch.as_deref(),
        )
        .await
        {
            Ok(p) => p,
            Err(e) => {
                eprintln!("❌ Failed to generate review pack: {e}");
                return ExitCode::from(k0maru_verifier::constants::EXIT_FATAL_ERROR as u8);
            }
        };

        if args.json {
            let json_pack = match serde_json::to_string_pretty(&review_pack) {
                Ok(j) => j,
                Err(e) => {
                    eprintln!("❌ Failed to serialize review pack: {e}");
                    return ExitCode::from(k0maru_verifier::constants::EXIT_FATAL_ERROR as u8);
                }
            };
            println!("{json_pack}");
        } else {
            println!("{}", review_pack.markdown_summary);
            println!("📜 Receipt written to {}", target_receipt_path.display());
        }
    } else {
        if args.json {
            println!("{json_receipt}");
        } else {
            println!("{}", format_pretty_summary(&receipt));
            println!("📜 Receipt written to {}", target_receipt_path.display());
        }
    }

    let code: u8 = match receipt.verdict {
        VerdictStatus::Success => k0maru_verifier::constants::EXIT_SUCCESS as u8,
        VerdictStatus::Failed => k0maru_verifier::constants::EXIT_VERIFICATION_FAILED as u8,
        VerdictStatus::Blocked => k0maru_verifier::constants::EXIT_FATAL_ERROR as u8,
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
        assert!(!resolved.review_pack);
    }

    #[test]
    fn test_cli_review_pack_subcommand_parsing() {
        let args = vec![
            "k0maru-verifier",
            "review-pack",
            "--workspace",
            "/tmp/demo-rp",
            "--task-id",
            "rp-1",
        ];
        let cli = Cli::try_parse_from(args).expect("parse review-pack subcommand");
        let resolved = cli.resolve();
        assert_eq!(resolved.workspace, PathBuf::from("/tmp/demo-rp"));
        assert_eq!(resolved.task_id.as_deref(), Some("rp-1"));
        assert!(resolved.review_pack);
    }

    #[test]
    fn test_cli_review_pack_flag_parsing() {
        let args = vec![
            "k0maru-verifier",
            "--review-pack",
            "--workspace",
            "/tmp/demo-flag",
        ];
        let cli = Cli::try_parse_from(args).expect("parse review-pack flag");
        let resolved = cli.resolve();
        assert!(resolved.review_pack);
        assert_eq!(resolved.workspace, PathBuf::from("/tmp/demo-flag"));
    }

    #[test]
    fn test_cli_run_alias_parsing() {
        let args = vec![
            "k0maru-verifier",
            "run",
            "--workspace",
            "/tmp/demo-run",
            "--task-id",
            "run-1",
        ];
        let cli = Cli::try_parse_from(args).expect("parse run alias");
        let resolved = cli.resolve();
        assert_eq!(resolved.workspace, PathBuf::from("/tmp/demo-run"));
        assert_eq!(resolved.task_id.as_deref(), Some("run-1"));
    }

    #[test]
    fn test_cli_baseline_subcommand_parsing() {
        let args = vec![
            "k0maru-verifier",
            "baseline",
            "--workspace",
            "/tmp/demo-base",
            "--base-branch",
            "main",
        ];
        let cli = Cli::try_parse_from(args).expect("parse baseline subcommand");
        let resolved = cli.resolve();
        assert_eq!(resolved.workspace, PathBuf::from("/tmp/demo-base"));
        assert_eq!(resolved.base_branch.as_deref(), Some("main"));
        assert!(resolved.immutable_tests);
    }

    #[test]
    fn test_cli_diff_subcommand_parsing() {
        let args = vec![
            "k0maru-verifier",
            "diff",
            "--workspace",
            "/tmp/demo-diff",
            "--whitelist",
            "src/",
        ];
        let cli = Cli::try_parse_from(args).expect("parse diff subcommand");
        let resolved = cli.resolve();
        assert_eq!(resolved.workspace, PathBuf::from("/tmp/demo-diff"));
        assert_eq!(resolved.whitelist, Some(vec!["src/".to_string()]));
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
        assert!(!resolved.review_pack);
    }

    #[test]
    fn test_verify_args_merge_fallback() {
        let root = VerifyArgs {
            workspace: PathBuf::from("/tmp/root-ws"),
            task_id: Some("root-task".to_string()),
            whitelist: Some(vec!["src/".to_string()]),
            out: Some(PathBuf::from("receipt.json")),
            json: true,
            runner: Some("cargo".to_string()),
            spec: Some(PathBuf::from("spec.json")),
            base_branch: Some("main".to_string()),
            immutable_tests: true,
            review_pack: true,
        };

        // When sub is default, all values fall back to root
        let sub_default = VerifyArgs::default();
        let merged = sub_default.merge_fallback(root.clone());
        assert_eq!(merged.workspace, PathBuf::from("/tmp/root-ws"));
        assert_eq!(merged.task_id.as_deref(), Some("root-task"));
        assert_eq!(merged.whitelist, Some(vec!["src/".to_string()]));
        assert_eq!(merged.out, Some(PathBuf::from("receipt.json")));
        assert!(merged.json);
        assert_eq!(merged.runner.as_deref(), Some("cargo"));
        assert_eq!(merged.spec, Some(PathBuf::from("spec.json")));
        assert_eq!(merged.base_branch.as_deref(), Some("main"));
        assert!(merged.immutable_tests);
        assert!(merged.review_pack);

        // When sub provides explicit values, they take precedence
        let sub_explicit = VerifyArgs {
            workspace: PathBuf::from("/tmp/sub-ws"),
            task_id: Some("sub-task".to_string()),
            json: false,
            ..Default::default()
        };
        let merged_explicit = sub_explicit.merge_fallback(root);
        assert_eq!(merged_explicit.workspace, PathBuf::from("/tmp/sub-ws"));
        assert_eq!(merged_explicit.task_id.as_deref(), Some("sub-task"));
        assert!(merged_explicit.json); // json is OR'd
        assert_eq!(merged_explicit.whitelist, Some(vec!["src/".to_string()]));
        assert_eq!(merged_explicit.base_branch.as_deref(), Some("main"));
        assert!(merged_explicit.immutable_tests);
        assert!(merged_explicit.review_pack);
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
                    message: None,
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
