use clap::Parser;
use k0maru_verifier::guard::ScopeGuard;
use k0maru_verifier::receipt::*;
use k0maru_verifier::runner::{CargoRunner, LanguageRunner};
use std::path::PathBuf;
use tokio::process::Command;

#[derive(Parser)]
#[command(name = "k0maru-verifier")]
#[command(about = "Deterministic test oracle and execution receipt signer for K0maru-Harness")]
struct Cli {
    #[arg(long, default_value = ".")]
    workspace: PathBuf,

    #[arg(long)]
    task_id: Option<String>,

    #[arg(long)]
    whitelist: Option<Vec<String>>,

    #[arg(long)]
    out: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let workspace = std::fs::canonicalize(&cli.workspace)?;
    let task_id = cli.task_id.unwrap_or_else(|| "task_ad_hoc".to_string());

    println!("⚖️ [K0maru-Verifier] Running deterministic physical checks on {:?}...", workspace);

    let runner = CargoRunner;
    if !runner.detect(&workspace) {
        eprintln!("⚠️ No supported runner detected for workspace {:?}", workspace);
    }

    let compile_res = runner.check(&workspace).await?;
    let test_res = runner.test(&workspace).await?;
    let lint_res = runner.lint(&workspace).await?;
    let scope_res = ScopeGuard::check_scope(&workspace, cli.whitelist.as_deref()).await?;

    // Compute Git Patch SHA-256
    let diff_output = Command::new("git")
        .args(["diff", "HEAD"])
        .current_dir(&workspace)
        .output()
        .await?;
    let git_patch_sha256 = ExecutionReceipt::compute_sha256(&diff_output.stdout);

    let is_all_passed = compile_res.status == StepStatus::Passed
        && test_res.status == StepStatus::Passed
        && lint_res.status == StepStatus::Passed
        && scope_res.status == StepStatus::Passed;

    let verdict = if is_all_passed {
        VerdictStatus::Success
    } else {
        VerdictStatus::Failed
    };

    let receipt = ExecutionReceipt {
        schema: DEFAULT_RECEIPT_SCHEMA.to_string(),
        task_id: task_id.clone(),
        workspace: workspace.clone(),
        verdict,
        checks: CheckSummary {
            compilation: compile_res,
            tests: test_res,
            linter: lint_res,
            scope_guard: scope_res,
        },
        artifacts: ArtifactSummary {
            git_patch_sha256,
            receipt_timestamp: chrono::Utc::now().to_rfc3339(),
        },
    };

    let json_receipt = serde_json::to_string_pretty(&receipt)?;
    if let Some(out_path) = cli.out {
        std::fs::write(&out_path, &json_receipt)?;
        println!("📜 Receipt written to {:?}", out_path);
    } else {
        println!("{}", json_receipt);
    }

    if !is_all_passed {
        std::process::exit(1);
    }

    println!("✅ [K0maru-Verifier] Verification passed with exit code 0.");
    Ok(())
}
