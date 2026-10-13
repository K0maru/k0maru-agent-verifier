use assert_cmd::Command;
use k0maru_verifier::constants::{DEFAULT_RECEIPT_SCHEMA, TUPLE_KIND_BLOCKED, TUPLE_KIND_VERIFIED};
use k0maru_verifier::receipt::*;
use k0maru_verifier::tuple::{LindaTuple, TupleEmitter, VerificationTuplePayload};
use predicates::prelude::*;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

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
    run_git(&["config", "user.name", "TupleTester"]);
    run_git(&["config", "user.email", "tester@tuple.com"]);
    std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "dummy_tuple"
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

fn sample_receipt(verdict: VerdictStatus) -> ExecutionReceipt {
    ExecutionReceipt {
        schema: DEFAULT_RECEIPT_SCHEMA.to_string(),
        task_id: "task-tuple-test".to_string(),
        workspace: PathBuf::from("/test/ws"),
        verdict,
        checks: CheckSummary {
            compilation: CompilationCheck {
                status: if verdict == VerdictStatus::Success {
                    StepStatus::Passed
                } else {
                    StepStatus::Failed
                },
                exit_code: if verdict == VerdictStatus::Success {
                    0
                } else {
                    1
                },
                message: None,
            },
            tests: TestCheck {
                status: StepStatus::Passed,
                exit_code: Some(0),
                total: 5,
                failed: 0,
                duration_ms: Some(50),
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
                modified_files: vec!["src/lib.rs".to_string()],
                unauthorized_files: Vec::new(),
                message: None,
            },
            diff_coverage: None,
        },
        artifacts: ArtifactSummary {
            git_patch_sha256: Some(
                "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
            ),
            receipt_timestamp: Some("2026-10-13T12:00:00Z".to_string()),
        },
    }
}

#[test]
fn test_tuple_emitter_unit_verified_and_blocked() {
    let temp_dir = TempDir::new().unwrap();
    let receipt_path = Path::new("/receipts/test.json");

    // Success outcome -> TASK_VERIFIED
    let success_receipt = sample_receipt(VerdictStatus::Success);
    let verified_tuple = TupleEmitter::emit_verification_outcome(
        Some(temp_dir.path()),
        Some("VERIFIER"),
        &success_receipt,
        receipt_path,
    )
    .expect("emit verified tuple");

    assert_eq!(verified_tuple.kind, TUPLE_KIND_VERIFIED);
    assert_eq!(verified_tuple.domain, "VERIFIER");
    assert_eq!(verified_tuple.task_id, "task-tuple-test");

    let payload: VerificationTuplePayload =
        serde_json::from_str(&verified_tuple.payload).expect("valid JSON payload");
    assert_eq!(payload.status, "PASS");
    assert_eq!(payload.verdict, VerdictStatus::Success);
    assert_eq!(payload.receipt_path, receipt_path);
    assert_eq!(
        payload.git_patch_sha256.as_deref(),
        Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
    );

    // Failure outcome -> TASK_BLOCKED
    let fail_receipt = sample_receipt(VerdictStatus::Failed);
    let blocked_tuple = TupleEmitter::emit_verification_outcome(
        Some(temp_dir.path()),
        Some("AUTH-SRV"),
        &fail_receipt,
        receipt_path,
    )
    .expect("emit blocked tuple");

    assert_eq!(blocked_tuple.kind, TUPLE_KIND_BLOCKED);
    assert_eq!(blocked_tuple.domain, "AUTH-SRV");
    let fail_payload: VerificationTuplePayload =
        serde_json::from_str(&blocked_tuple.payload).expect("valid JSON payload");
    assert_eq!(fail_payload.status, "FAILED");
    assert_eq!(fail_payload.verdict, VerdictStatus::Failed);
    assert!(fail_payload.failure_reason.is_some());
}

#[test]
fn test_cli_automatic_tuple_emission_on_success() {
    let ws = setup_temp_cargo_workspace();
    let temp_tuples = TempDir::new().unwrap();

    let mut cmd = Command::cargo_bin("k0maru-verifier").unwrap();
    cmd.args([
        "verify",
        "--workspace",
        ws.path().to_str().unwrap(),
        "--task-id",
        "task-tuple-cli-01",
        "--domain",
        "ROUTER-FABRIC",
        "--tuple-dir",
        temp_tuples.path().to_str().unwrap(),
    ])
    .assert()
    .success()
    .stdout(predicate::str::contains("Linda Tuple emitted:"))
    .stdout(predicate::str::contains("TASK_VERIFIED"));

    // Verify tuple file exists on disk
    let entries: Vec<_> = std::fs::read_dir(temp_tuples.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(entries.len(), 1, "Exactly one tuple should be emitted");

    let content = std::fs::read_to_string(entries[0].path()).unwrap();
    let tuple: LindaTuple = serde_json::from_str(&content).expect("parse emitted tuple");
    assert_eq!(tuple.kind, TUPLE_KIND_VERIFIED);
    assert_eq!(tuple.domain, "ROUTER-FABRIC");
    assert_eq!(tuple.task_id, "task-tuple-cli-01");

    let payload: VerificationTuplePayload =
        serde_json::from_str(&tuple.payload).expect("parse tuple payload");
    assert_eq!(payload.status, "PASS");
    assert_eq!(payload.verdict, VerdictStatus::Success);
    assert!(payload
        .receipt_path
        .to_str()
        .unwrap()
        .contains("task-tuple-cli-01.json"));
}

#[test]
fn test_cli_tuple_emission_blocked_on_failure() {
    let ws = setup_temp_cargo_workspace();
    let temp_tuples = TempDir::new().unwrap();

    // Introduce failing test
    std::fs::write(
        ws.path().join("src/lib.rs"),
        "pub fn ok() -> bool { false }\n#[test] fn t() { assert!(ok()); }\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("k0maru-verifier").unwrap();
    cmd.args([
        "--workspace",
        ws.path().to_str().unwrap(),
        "--task-id",
        "task-fail-tuple-02",
        "--tuple-dir",
        temp_tuples.path().to_str().unwrap(),
    ])
    .assert()
    .code(1)
    .stdout(predicate::str::contains("TASK_BLOCKED"));

    let entries: Vec<_> = std::fs::read_dir(temp_tuples.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(entries.len(), 1);

    let content = std::fs::read_to_string(entries[0].path()).unwrap();
    let tuple: LindaTuple = serde_json::from_str(&content).expect("parse emitted tuple");
    assert_eq!(tuple.kind, TUPLE_KIND_BLOCKED);
    assert_eq!(tuple.task_id, "task-fail-tuple-02");

    let payload: VerificationTuplePayload =
        serde_json::from_str(&tuple.payload).expect("parse tuple payload");
    assert_eq!(payload.status, "FAILED");
    assert_eq!(payload.verdict, VerdictStatus::Failed);
    assert!(payload.failure_reason.is_some());
}

#[test]
fn test_cli_no_tuple_flag_suppresses_emission() {
    let ws = setup_temp_cargo_workspace();
    let temp_tuples = TempDir::new().unwrap();

    let mut cmd = Command::cargo_bin("k0maru-verifier").unwrap();
    cmd.args([
        "verify",
        "--workspace",
        ws.path().to_str().unwrap(),
        "--task-id",
        "task-no-tuple-03",
        "--no-tuple",
        "--tuple-dir",
        temp_tuples.path().to_str().unwrap(),
    ])
    .assert()
    .success()
    .stdout(predicate::str::contains("Linda Tuple emitted").not());

    let entries: Vec<_> = std::fs::read_dir(temp_tuples.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(
        entries.len(),
        0,
        "No tuple file should be written when --no-tuple is set"
    );
}
