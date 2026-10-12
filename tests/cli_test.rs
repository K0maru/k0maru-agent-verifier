use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::{NamedTempFile, TempDir};

fn setup_temp_git_workspace() -> TempDir {
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
    std::fs::write(root.join("README.md"), "# Test Workspace").unwrap();
    run_git(&["add", "README.md"]);
    run_git(&["commit", "-m", "init"]);
    temp
}

#[test]
fn test_cli_help() {
    let mut cmd = Command::cargo_bin("k0maru-verifier").unwrap();
    cmd.arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Deterministic test oracle"));
}

#[test]
fn test_cli_verify_subcommand_help() {
    let mut cmd = Command::cargo_bin("k0maru-verifier").unwrap();
    cmd.args(["verify", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Verify workspace quality gates"));
}

#[test]
fn test_cli_non_existent_workspace_fails_with_code_2() {
    let mut cmd = Command::cargo_bin("k0maru-verifier").unwrap();
    cmd.args(["--workspace", "/non_existent_path_999999"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("Fatal verification error"));
}

#[test]
fn test_cli_spec_file_integration() {
    let spec = NamedTempFile::new().unwrap();
    let spec_json = r#"
{
  "task_id": "integration-spec-42",
  "runner": "non_existent_runner_name"
}
"#;
    std::fs::write(spec.path(), spec_json).unwrap();

    let mut cmd = Command::cargo_bin("k0maru-verifier").unwrap();
    cmd.args(["verify", "--spec", spec.path().to_str().unwrap()])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "Runner 'non_existent_runner_name' not found",
        ));
}

#[test]
fn test_cli_workspace_verification_json() {
    let temp = setup_temp_git_workspace();
    let mut cmd = Command::cargo_bin("k0maru-verifier").unwrap();
    let assert = cmd
        .args([
            "--workspace",
            temp.path().to_str().unwrap(),
            "--task-id",
            "test-temp-repo",
            "--json",
        ])
        .assert();

    let output = assert.get_output();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let receipt: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout must be valid JSON");

    assert_eq!(receipt["task_id"], "test-temp-repo");
    assert!(receipt["verdict"].is_string());
    assert!(receipt["checks"]["compilation"].is_object());
    assert!(receipt["checks"]["tests"].is_object());
    assert!(receipt["checks"]["linter"].is_object());
    assert!(receipt["checks"]["scope_guard"].is_object());
}

#[test]
fn test_cli_receipt_out_flag() {
    let temp = setup_temp_git_workspace();
    let receipt_out = NamedTempFile::new().unwrap();
    let mut cmd = Command::cargo_bin("k0maru-verifier").unwrap();
    let assert = cmd
        .args([
            "verify",
            "--workspace",
            temp.path().to_str().unwrap(),
            "--task-id",
            "test-out-flag",
            "--out",
            receipt_out.path().to_str().unwrap(),
            "--json",
        ])
        .assert();

    let _ = assert;
    let content = std::fs::read_to_string(receipt_out.path()).expect("read receipt file");
    let receipt: serde_json::Value =
        serde_json::from_str(&content).expect("receipt file must be valid JSON");
    assert_eq!(receipt["task_id"], "test-out-flag");
}

fn setup_temp_cargo_workspace() -> TempDir {
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
    std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "dummy_cli"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/lib.rs"), "pub fn ok() -> bool { true }\n").unwrap();
    run_git(&["add", "."]);
    run_git(&["commit", "-m", "init cargo"]);
    temp
}

#[test]
fn test_cli_default_receipt_emission_when_out_omitted() {
    let temp_ws = setup_temp_cargo_workspace();
    let temp_receipts_dir = TempDir::new().unwrap();

    let mut cmd = Command::cargo_bin("k0maru-verifier").unwrap();
    cmd.env("K0MARU_RECEIPT_DIR", temp_receipts_dir.path())
        .args([
            "verify",
            "--workspace",
            temp_ws.path().to_str().unwrap(),
            "--task-id",
            "task-auto-store-007",
            "--json",
        ])
        .assert()
        .code(0);

    let expected_receipt_file = temp_receipts_dir.path().join("task-auto-store-007.json");
    assert!(
        expected_receipt_file.is_file(),
        "Receipt file must automatically be written to standard receipt store when --out is omitted"
    );

    let content =
        std::fs::read_to_string(&expected_receipt_file).expect("read auto-stored receipt");
    let receipt: serde_json::Value = serde_json::from_str(&content).expect("valid JSON receipt");
    assert_eq!(receipt["task_id"], "task-auto-store-007");
    assert_eq!(receipt["verdict"], "success");
}

#[test]
fn test_cli_immutable_tests_anti_tampering_blocks_execution() {
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

    run_git(&["init", "-b", "main"]);
    run_git(&["config", "user.name", "Tester"]);
    run_git(&["config", "user.email", "tester@example.com"]);

    // Create Cargo.toml and baseline test
    std::fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "dummy"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        "pub fn add(a: i32, b: i32) -> i32 { a + b }\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("tests")).unwrap();
    std::fs::write(
        root.join("tests/baseline_test.rs"),
        "#[test]\nfn test_baseline() {\n    assert_eq!(dummy::add(1, 1), 2);\n    assert_ne!(1, 2);\n}\n",
    )
    .unwrap();

    run_git(&["add", "."]);
    run_git(&["commit", "-m", "baseline with test assertions"]);

    // Create feature branch and tamper with existing test (delete assertion)
    run_git(&["checkout", "-b", "feat/tamper"]);
    std::fs::write(
        root.join("tests/baseline_test.rs"),
        "#[test]\nfn test_baseline() {\n    assert_eq!(dummy::add(1, 1), 2);\n}\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("k0maru-verifier").unwrap();
    cmd.args([
        "verify",
        "--workspace",
        root.to_str().unwrap(),
        "--task-id",
        "task-tamper-check",
        "--base-branch",
        "main",
        "--immutable-tests",
    ])
    .assert()
    .code(1) // EXIT_VERIFICATION_FAILED
    .stdout(predicate::str::contains(
        "Anti-tampering violation: deletions detected in existing test baseline",
    ));
}
