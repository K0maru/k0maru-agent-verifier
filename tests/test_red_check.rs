use assert_cmd::Command;
use k0maru_verifier::receipt::*;
use k0maru_verifier::verifier::{VerifierConfig, WorkspaceVerifier};
use std::fs;
use std::path::Path;
use std::process::Command as StdCommand;
use tempfile::TempDir;

fn run_git(workspace: &Path, args: &[&str]) {
    let status = StdCommand::new("git")
        .args(args)
        .current_dir(workspace)
        .output()
        .expect("git execution failed");
    assert!(
        status.status.success(),
        "git command failed: {:?} (stderr: {})",
        args,
        String::from_utf8_lossy(&status.stderr)
    );
}

fn init_git_repo(workspace: &Path) {
    run_git(workspace, &["init"]);
    run_git(workspace, &["config", "user.name", "K0maru Bot"]);
    run_git(workspace, &["config", "user.email", "bot@k0maru.dev"]);
    fs::write(workspace.join(".gitignore"), "target/\n").unwrap();
}

#[tokio::test]
async fn test_red_check_genuine_test_succeeds_falsification() {
    let temp = TempDir::new().unwrap();
    let ws = temp.path();
    init_git_repo(ws);

    // Initial baseline commit: Cargo.toml and src/lib.rs with basic add()
    fs::write(
        ws.join("Cargo.toml"),
        r#"[package]
name = "dummy-falsify"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();

    fs::create_dir_all(ws.join("src")).unwrap();
    fs::write(
        ws.join("src/lib.rs"),
        r#"pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
"#,
    )
    .unwrap();

    run_git(ws, &["add", "."]);
    run_git(ws, &["commit", "-m", "initial baseline"]);

    // Now developer adds a new function multiply() and a new test for it
    fs::write(
        ws.join("src/lib.rs"),
        r#"pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

pub fn multiply(a: i32, b: i32) -> i32 {
    a * b
}
"#,
    )
    .unwrap();

    fs::create_dir_all(ws.join("tests")).unwrap();
    fs::write(
        ws.join("tests/test_multiply.rs"),
        r#"use dummy_falsify::multiply;

#[test]
fn test_multiply() {
    assert_eq!(multiply(3, 4), 12);
}
"#,
    )
    .unwrap();

    let verifier = WorkspaceVerifier::new();
    let config = VerifierConfig::new(ws, "TASK-GENUINE-TEST")
        .red_check(true)
        .auto_detect(true);

    let receipt = verifier.verify(&config).await.expect("verification ok");

    assert_eq!(receipt.verdict, VerdictStatus::Success);
    let red_check = receipt.checks.red_check.expect("red_check present");
    assert_eq!(red_check.status, StepStatus::Passed);
    assert!(red_check
        .test_files
        .iter()
        .any(|f| f.contains("test_multiply.rs")));
    assert!(red_check.prod_files.iter().any(|f| f.contains("lib.rs")));
    assert!(red_check
        .message
        .unwrap()
        .contains("falsification confirmed"));
}

#[tokio::test]
async fn test_red_check_fake_green_test_fails_falsification() {
    let temp = TempDir::new().unwrap();
    let ws = temp.path();
    init_git_repo(ws);

    // Initial baseline commit
    fs::write(
        ws.join("Cargo.toml"),
        r#"[package]
name = "dummy-fake-green"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();

    fs::create_dir_all(ws.join("src")).unwrap();
    fs::write(
        ws.join("src/lib.rs"),
        r#"pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
"#,
    )
    .unwrap();

    run_git(ws, &["add", "."]);
    run_git(ws, &["commit", "-m", "initial baseline"]);

    // Developer modifies src/lib.rs, but writes a tautological/fake-green test that passes on baseline!
    fs::write(
        ws.join("src/lib.rs"),
        r#"pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

pub fn greet() -> &'static str {
    "hello"
}
"#,
    )
    .unwrap();

    fs::create_dir_all(ws.join("tests")).unwrap();
    fs::write(
        ws.join("tests/test_tautology.rs"),
        r#"#[test]
fn test_tautological_fake_green() {
    assert_eq!(2 + 2, 4);
}
"#,
    )
    .unwrap();

    let verifier = WorkspaceVerifier::new();
    let config = VerifierConfig::new(ws, "TASK-FAKE-GREEN")
        .red_check(true)
        .auto_detect(true);

    let receipt = verifier
        .verify(&config)
        .await
        .expect("verification receipt generated");

    // Verdict must be Failed because red_check detected fake green test!
    assert_eq!(receipt.verdict, VerdictStatus::Failed);
    let red_check = receipt.checks.red_check.expect("red_check present");
    assert_eq!(red_check.status, StepStatus::Failed);
    assert!(red_check
        .message
        .unwrap()
        .contains("fake green test detected"));
}

#[tokio::test]
async fn test_red_check_skipped_when_no_tests_modified() {
    let temp = TempDir::new().unwrap();
    let ws = temp.path();
    init_git_repo(ws);

    fs::write(
        ws.join("Cargo.toml"),
        r#"[package]
name = "dummy-no-test"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();

    fs::create_dir_all(ws.join("src")).unwrap();
    fs::write(
        ws.join("src/lib.rs"),
        r#"pub fn answer() -> i32 {
    42
}
"#,
    )
    .unwrap();

    run_git(ws, &["add", "."]);
    run_git(ws, &["commit", "-m", "initial"]);

    // Modify only production code without adding any tests
    fs::write(
        ws.join("src/lib.rs"),
        r#"pub fn answer() -> i32 {
    42 // updated comment
}
"#,
    )
    .unwrap();

    let verifier = WorkspaceVerifier::new();
    let config = VerifierConfig::new(ws, "TASK-NO-TESTS")
        .red_check(true)
        .auto_detect(true);

    let receipt = verifier.verify(&config).await.expect("verification ok");
    let red_check = receipt.checks.red_check.expect("red_check present");
    assert_eq!(red_check.status, StepStatus::Skipped);
    assert_eq!(receipt.verdict, VerdictStatus::Success);
}

#[test]
fn test_cli_red_check_flag_and_subcommand() {
    let temp = TempDir::new().unwrap();
    let ws = temp.path();
    init_git_repo(ws);

    fs::write(
        ws.join("Cargo.toml"),
        r#"[package]
name = "dummy-cli-red"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();

    fs::create_dir_all(ws.join("src")).unwrap();
    fs::write(
        ws.join("src/lib.rs"),
        r#"pub fn foo() -> bool { true }
"#,
    )
    .unwrap();

    run_git(ws, &["add", "."]);
    run_git(ws, &["commit", "-m", "baseline"]);

    // Genuine addition
    fs::write(
        ws.join("src/lib.rs"),
        r#"pub fn foo() -> bool { true }
pub fn bar() -> bool { false }
"#,
    )
    .unwrap();

    fs::create_dir_all(ws.join("tests")).unwrap();
    fs::write(
        ws.join("tests/test_bar.rs"),
        r#"use dummy_cli_red::bar;

#[test]
fn test_bar() {
    assert!(!bar());
}
"#,
    )
    .unwrap();

    // 1. Verify via flag --red-check
    let mut cmd = Command::cargo_bin("k0maru-verifier").unwrap();
    cmd.arg("verify")
        .arg("--workspace")
        .arg(ws)
        .arg("--red-check")
        .arg("--json");

    let assert = cmd.assert().success();
    let output = assert.get_output();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let val: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON stdout");
    assert_eq!(val["verdict"], "success");
    assert_eq!(val["checks"]["red_check"]["status"], "passed");
    assert_eq!(
        val["checks"]["red_check"]["prod_files"],
        serde_json::json!(["src/lib.rs"])
    );
    assert_eq!(
        val["checks"]["red_check"]["test_files"],
        serde_json::json!(["tests/test_bar.rs"])
    );

    // 2. Verify via subcommand red-check
    let mut sub_cmd = Command::cargo_bin("k0maru-verifier").unwrap();
    sub_cmd
        .arg("red-check")
        .arg("--workspace")
        .arg(ws)
        .arg("--json");

    let sub_assert = sub_cmd.assert().success();
    let sub_output = sub_assert.get_output();
    let sub_stdout = String::from_utf8_lossy(&sub_output.stdout);
    let sub_val: serde_json::Value =
        serde_json::from_str(&sub_stdout).expect("valid JSON subcommand stdout");
    assert_eq!(sub_val["verdict"], "success");
    assert_eq!(sub_val["checks"]["red_check"]["status"], "passed");
}

#[tokio::test]
async fn test_red_check_rejects_compile_error_masquerade_with_tautological_assert_true() {
    let temp = TempDir::new().unwrap();
    let ws = temp.path();
    init_git_repo(ws);

    fs::write(
        ws.join("Cargo.toml"),
        r#"[package]
name = "dummy-masquerade"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();

    fs::create_dir_all(ws.join("src")).unwrap();
    fs::write(
        ws.join("src/lib.rs"),
        r#"pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
"#,
    )
    .unwrap();

    run_git(ws, &["add", "."]);
    run_git(ws, &["commit", "-m", "initial baseline"]);

    // Agent adds a new function `new_calc` in src/lib.rs, and writes a tautological test
    // that references `new_calc` (so baseline fails to compile) but only asserts `assert!(true)`.
    fs::write(
        ws.join("src/lib.rs"),
        r#"pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

pub fn new_calc() -> i32 {
    42
}
"#,
    )
    .unwrap();

    fs::create_dir_all(ws.join("tests")).unwrap();
    fs::write(
        ws.join("tests/fake_test.rs"),
        r#"#[test]
fn test_fake() {
    let _ = dummy_masquerade::new_calc();
    assert!(true);
}
"#,
    )
    .unwrap();

    let verifier = WorkspaceVerifier::new();
    let config = VerifierConfig::new(ws, "TASK-MASQUERADE")
        .red_check(true)
        .auto_detect(true);

    let receipt = verifier
        .verify(&config)
        .await
        .expect("verification receipt generated");

    assert_eq!(receipt.verdict, VerdictStatus::Failed);
    let red_check = receipt.checks.red_check.expect("red_check present");
    assert_eq!(red_check.status, StepStatus::Failed);
    let msg = red_check.message.unwrap();
    assert!(
        msg.contains("Tautological assertion detected"),
        "unexpected red_check message: {msg}"
    );
}

#[tokio::test]
async fn test_red_check_rejects_hollow_test_without_assertions() {
    let temp = TempDir::new().unwrap();
    let ws = temp.path();
    init_git_repo(ws);

    fs::write(
        ws.join("Cargo.toml"),
        r#"[package]
name = "dummy-hollow"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();

    fs::create_dir_all(ws.join("src")).unwrap();
    fs::write(
        ws.join("src/lib.rs"),
        r#"pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
"#,
    )
    .unwrap();

    run_git(ws, &["add", "."]);
    run_git(ws, &["commit", "-m", "initial baseline"]);

    fs::write(
        ws.join("src/lib.rs"),
        r#"pub fn add(a: i32, b: i32) -> i32 {
    a + b + 0
}
"#,
    )
    .unwrap();

    fs::create_dir_all(ws.join("tests")).unwrap();
    fs::write(
        ws.join("tests/hollow_test.rs"),
        r#"use dummy_hollow::add;

#[test]
fn t() {
    let _ = add(1, 2);
}
"#,
    )
    .unwrap();

    let verifier = WorkspaceVerifier::new();
    let config = VerifierConfig::new(ws, "TASK-HOLLOW")
        .red_check(true)
        .auto_detect(true);

    let receipt = verifier
        .verify(&config)
        .await
        .expect("verification receipt generated");

    assert_eq!(receipt.verdict, VerdictStatus::Failed);
    let red_check = receipt.checks.red_check.expect("red_check present");
    assert_eq!(red_check.status, StepStatus::Failed);
    let msg = red_check.message.unwrap();
    assert!(
        msg.contains("Hollow test detected"),
        "unexpected red_check message: {msg}"
    );
}
