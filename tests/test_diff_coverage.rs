//! Integration tests for Phase 4: Diff Coverage Gate.

use assert_cmd::Command;
use k0maru_verifier::constants::EXIT_VERIFICATION_FAILED;
use k0maru_verifier::receipt::*;
use k0maru_verifier::verifier::{VerifierConfig, WorkspaceVerifier};
use std::fs;
use std::process::Command as StdCommand;
use tempfile::TempDir;

fn setup_git_workspace_with_diff() -> (TempDir, String) {
    let temp = TempDir::new().expect("create temp dir");
    let ws = temp.path();

    // 1. git init
    assert!(StdCommand::new("git")
        .args(["init"])
        .current_dir(ws)
        .status()
        .unwrap()
        .success());
    assert!(StdCommand::new("git")
        .args(["config", "user.name", "K0maru Agent"])
        .current_dir(ws)
        .status()
        .unwrap()
        .success());
    assert!(StdCommand::new("git")
        .args(["config", "user.email", "agent@k0maru.dev"])
        .current_dir(ws)
        .status()
        .unwrap()
        .success());

    // 2. Initial commit with Cargo.toml and src/lib.rs
    let cargo_toml = r#"
[package]
name = "demo-cov"
version = "0.1.0"
edition = "2021"
"#;
    fs::write(ws.join("Cargo.toml"), cargo_toml).unwrap();
    fs::create_dir_all(ws.join("src")).unwrap();

    let initial_lib = r#"
#[allow(dead_code)]
pub fn base_calc(a: i32) -> i32 {
    a * 2
}
"#;
    fs::write(ws.join("src/lib.rs"), initial_lib).unwrap();

    assert!(StdCommand::new("git")
        .args(["add", "."])
        .current_dir(ws)
        .status()
        .unwrap()
        .success());
    assert!(StdCommand::new("git")
        .args(["commit", "-m", "initial commit"])
        .current_dir(ws)
        .status()
        .unwrap()
        .success());

    // 3. Make modifications adding new source lines in src/lib.rs
    let modified_lib = r#"
#[allow(dead_code)]
pub fn base_calc(a: i32) -> i32 {
    a * 2
}

#[allow(dead_code)]
pub fn new_feature(x: i32) -> i32 {
    (x + 1) * 2
}
"#;
    fs::write(ws.join("src/lib.rs"), modified_lib).unwrap();

    (temp, "src/lib.rs".to_string())
}

#[tokio::test]
async fn test_diff_coverage_passes_when_meeting_threshold() {
    let (repo, file_path) = setup_git_workspace_with_diff();
    let ws = repo.path();

    // Create lcov.info covering lines 6, 7, 8, 9 in src/lib.rs
    let lcov = format!("TN:\nSF:{file_path}\nDA:6,1\nDA:7,1\nDA:8,1\nDA:9,1\nend_of_record\n");
    fs::write(ws.join("lcov.info"), lcov).unwrap();

    let config = VerifierConfig::new(ws, "task-diff-cov-pass")
        .min_diff_coverage(75.0)
        .coverage_file(ws.join("lcov.info"));

    let verifier = WorkspaceVerifier::new();
    let receipt = verifier.verify(&config).await.expect("verify succeeds");

    assert_eq!(receipt.verdict, VerdictStatus::Success);
    assert!(receipt.is_passing());

    let cov_check = receipt
        .checks
        .diff_coverage
        .expect("must have diff coverage check");
    assert_eq!(cov_check.status, StepStatus::Passed);
    assert_eq!(cov_check.min_threshold, Some(75.0));
    assert!(cov_check.coverage_pct.unwrap() >= 75.0);
    assert!(cov_check.covered_lines > 0);
}

#[tokio::test]
async fn test_diff_coverage_fails_when_below_threshold() {
    let (repo, file_path) = setup_git_workspace_with_diff();
    let ws = repo.path();

    // Create lcov.info covering only 1 line out of 4 added lines (25% coverage)
    let lcov = format!("TN:\nSF:{file_path}\nDA:6,1\nDA:7,0\nDA:8,0\nDA:9,0\nend_of_record\n");
    fs::write(ws.join("lcov.info"), lcov).unwrap();

    let config = VerifierConfig::new(ws, "task-diff-cov-fail")
        .min_diff_coverage(80.0)
        .coverage_file(ws.join("lcov.info"));

    let verifier = WorkspaceVerifier::new();
    let receipt = verifier.verify(&config).await.expect("verify succeeds");

    assert_eq!(receipt.verdict, VerdictStatus::Failed);
    assert!(!receipt.is_passing());

    let cov_check = receipt
        .checks
        .diff_coverage
        .expect("must have diff coverage check");
    assert_eq!(cov_check.status, StepStatus::Failed);
    assert_eq!(cov_check.min_threshold, Some(80.0));
    assert!(cov_check.coverage_pct.unwrap() < 80.0);
    assert!(cov_check
        .message
        .unwrap()
        .contains("is below required threshold"));
}

#[tokio::test]
async fn test_diff_coverage_fails_when_report_missing() {
    let (repo, _) = setup_git_workspace_with_diff();
    let ws = repo.path();

    let config = VerifierConfig::new(ws, "task-missing-cov").min_diff_coverage(80.0);

    let verifier = WorkspaceVerifier::new();
    let receipt = verifier.verify(&config).await.expect("verify succeeds");

    assert_eq!(receipt.verdict, VerdictStatus::Failed);
    let cov_check = receipt
        .checks
        .diff_coverage
        .expect("must have diff coverage check");
    assert_eq!(cov_check.status, StepStatus::Failed);
    assert!(cov_check
        .message
        .unwrap()
        .contains("Coverage report not found"));
}

#[test]
fn test_cli_diff_coverage_gate_integration() {
    let (repo, file_path) = setup_git_workspace_with_diff();
    let ws = repo.path();

    // Create LCOV file with 100% coverage
    let lcov = format!("TN:\nSF:{file_path}\nDA:6,2\nDA:7,2\nDA:8,2\nDA:9,2\nend_of_record\n");
    fs::write(ws.join("coverage.lcov"), lcov).unwrap();

    let mut cmd = Command::cargo_bin("k0maru-verifier").unwrap();
    cmd.args([
        "diff-coverage",
        "--workspace",
        ws.to_str().unwrap(),
        "--task-id",
        "task-cli-cov-pass",
        "--min-diff-coverage",
        "80.0",
        "--coverage-file",
        ws.join("coverage.lcov").to_str().unwrap(),
    ])
    .assert()
    .success()
    .stdout(predicates::str::contains("SUCCESS"))
    .stdout(predicates::str::contains("Diff Coverage: passed"));
}

#[test]
fn test_cli_diff_coverage_gate_failure_exit_code() {
    let (repo, file_path) = setup_git_workspace_with_diff();
    let ws = repo.path();

    // 0% covered
    let lcov = format!("TN:\nSF:{file_path}\nDA:6,0\nDA:7,0\nDA:8,0\nend_of_record\n");
    fs::write(ws.join("coverage.lcov"), lcov).unwrap();

    let mut cmd = Command::cargo_bin("k0maru-verifier").unwrap();
    cmd.args([
        "--workspace",
        ws.to_str().unwrap(),
        "--task-id",
        "task-cli-cov-fail",
        "--min-diff-coverage",
        "80.0",
        "--coverage-file",
        ws.join("coverage.lcov").to_str().unwrap(),
    ])
    .assert()
    .code(EXIT_VERIFICATION_FAILED)
    .stdout(predicates::str::contains("FAILED"))
    .stdout(predicates::str::contains("Diff Coverage: failed"));
}
