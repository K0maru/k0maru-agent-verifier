use assert_cmd::Command;
use k0maru_verifier::receipt::*;
use k0maru_verifier::ReviewPack;
use predicates::prelude::*;
use std::path::PathBuf;
use tempfile::TempDir;

fn sample_receipt() -> ExecutionReceipt {
    ExecutionReceipt {
        schema: DEFAULT_RECEIPT_SCHEMA.to_string(),
        task_id: "task-rp-serde".to_string(),
        workspace: PathBuf::from("/test/ws"),
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
                total: 5,
                failed: 0,
                duration_ms: Some(42),
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
            scope_guard: ScopeGuardCheck {
                status: StepStatus::Passed,
                modified_files: vec!["src/a.rs".to_string(), "src/b.rs".to_string()],
                unauthorized_files: Vec::new(),
                message: None,
            },
            diff_coverage: None,
        },
        artifacts: ArtifactSummary {
            git_patch_sha256: Some(
                "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
            ),
            receipt_timestamp: Some("2026-10-12T12:00:00Z".to_string()),
        },
    }
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

    run_git(&["init", "-b", "main"]);
    run_git(&["config", "user.name", "ReviewTester"]);
    run_git(&["config", "user.email", "tester@review.com"]);
    std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "dummy_rp"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/lib.rs"), "pub fn ok() -> bool { true }\n").unwrap();
    run_git(&["add", "."]);
    run_git(&["commit", "-m", "init dummy"]);
    temp
}

#[test]
fn test_review_pack_serde_roundtrip() {
    let receipt = sample_receipt();
    let pack = ReviewPack::from_receipt_with_diff(
        receipt.clone(),
        " src/a.rs | 1 +\n 1 file changed, 1 insertion(+)".to_string(),
    );

    let serialized = serde_json::to_string_pretty(&pack).expect("serialize ReviewPack");
    let deserialized: ReviewPack =
        serde_json::from_str(&serialized).expect("deserialize ReviewPack");

    assert_eq!(pack, deserialized);
    assert_eq!(deserialized.task_id, "task-rp-serde");
    assert_eq!(deserialized.verdict, VerdictStatus::Success);
    assert_eq!(deserialized.changed_files, vec!["src/a.rs", "src/b.rs"]);
    assert!(deserialized.diff_stat.contains("src/a.rs"));
    assert!(deserialized.markdown_summary.lines().count() < 20);
}

#[tokio::test]
async fn test_review_pack_generate_in_real_git_repo() {
    let temp = setup_temp_cargo_workspace();
    let root = temp.path();

    // Modify src/lib.rs and create src/extra.rs
    std::fs::write(
        root.join("src/lib.rs"),
        "pub fn ok() -> bool { true }\npub fn added() -> i32 { 100 }\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src/extra.rs"),
        "pub fn helper() -> bool { false }\n",
    )
    .unwrap();

    let mut receipt = sample_receipt();
    receipt.workspace = root.to_path_buf();
    receipt.task_id = "task-real-git-01".to_string();
    receipt.checks.scope_guard.modified_files =
        vec!["src/extra.rs".to_string(), "src/lib.rs".to_string()];

    let pack = ReviewPack::generate(receipt, Some("main"))
        .await
        .expect("generate review pack");

    assert_eq!(pack.task_id, "task-real-git-01");
    assert_eq!(pack.workspace, root);
    assert_eq!(pack.verdict, VerdictStatus::Success);
    assert_eq!(pack.changed_files, vec!["src/extra.rs", "src/lib.rs"]);
    assert!(
        pack.diff_stat.contains("src/lib.rs"),
        "diff_stat should contain modified src/lib.rs"
    );
    assert!(
        pack.markdown_summary.lines().count() < 20,
        "markdown summary must be under 20 lines"
    );
    assert!(pack
        .markdown_summary
        .contains("### ⚖️ Review Pack: task-real-git-01"));
    assert!(pack.markdown_summary.contains("✅ Success"));
    assert!(pack.markdown_summary.contains("**Changed Files (2)**:"));
}

#[test]
fn test_cli_execution_review_pack_flag_success() {
    let temp = setup_temp_cargo_workspace();
    let root = temp.path();

    // Make a valid modification
    std::fs::write(
        root.join("src/lib.rs"),
        "pub fn ok() -> bool { true }\npub fn hello() {}\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("k0maru-verifier").unwrap();
    cmd.args([
        "--workspace",
        root.to_str().unwrap(),
        "--task-id",
        "task-cli-rp-flag",
        "--review-pack",
    ])
    .assert()
    .success()
    .stdout(predicate::str::contains(
        "### ⚖️ Review Pack: task-cli-rp-flag",
    ))
    .stdout(predicate::str::contains("✅ Success"))
    .stdout(predicate::str::contains("Changed Files"))
    .stdout(predicate::str::contains("📜 Receipt written to"));
}

#[test]
fn test_cli_execution_review_pack_subcommand() {
    let temp = setup_temp_cargo_workspace();
    let root = temp.path();

    std::fs::write(
        root.join("src/lib.rs"),
        "pub fn ok() -> bool { true }\npub fn sub() -> i32 { 1 }\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("k0maru-verifier").unwrap();
    cmd.args([
        "review-pack",
        "--workspace",
        root.to_str().unwrap(),
        "--task-id",
        "task-cli-sub-01",
    ])
    .assert()
    .success()
    .stdout(predicate::str::contains(
        "### ⚖️ Review Pack: task-cli-sub-01",
    ))
    .stdout(predicate::str::contains("✅ Success"))
    .stdout(predicate::str::contains("📜 Receipt written to"));
}

#[test]
fn test_cli_execution_review_pack_json() {
    let temp = setup_temp_cargo_workspace();
    let root = temp.path();

    std::fs::write(
        root.join("src/lib.rs"),
        "pub fn ok() -> bool { true }\npub fn json_fn() {}\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("k0maru-verifier").unwrap();
    let assert = cmd
        .args([
            "--review-pack",
            "--json",
            "--workspace",
            root.to_str().unwrap(),
            "--task-id",
            "task-json-01",
        ])
        .assert()
        .success();

    let output = assert.get_output();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout must be valid JSON ReviewPack");

    assert_eq!(json["task_id"], "task-json-01");
    assert_eq!(json["verdict"], "success");
    assert!(json["changed_files"].is_array());
    assert!(json["diff_stat"].is_string());
    assert!(json["markdown_summary"].is_string());
    assert!(json["receipt"]["checks"].is_object());
    assert_eq!(json["receipt"]["verdict"], "success");
}

#[test]
fn test_cli_execution_review_pack_failure_exit_code_and_diagnostics() {
    let temp = setup_temp_cargo_workspace();
    let root = temp.path();

    // Introduce a failing unit test
    std::fs::write(
        root.join("src/lib.rs"),
        r#"pub fn ok() -> bool { true }

#[cfg(test)]
mod tests {
    #[test]
    fn test_failing() {
        assert_eq!(1, 2, "intentional test failure");
    }
}
"#,
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("k0maru-verifier").unwrap();
    cmd.args([
        "--review-pack",
        "--workspace",
        root.to_str().unwrap(),
        "--task-id",
        "task-fail-rp-01",
    ])
    .assert()
    .code(1) // EXIT_VERIFICATION_FAILED
    .stdout(predicate::str::contains("FAILED"))
    .stdout(predicate::str::contains("Tests:"));
}
