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
