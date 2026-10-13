use k0maru_verifier::error::VerifierError;
use k0maru_verifier::receipt::{
    ArtifactSummary, CheckSummary, CompilationCheck, ExecutionReceipt, LinterCheck,
    ScopeGuardCheck, StepResult, StepStatus, TestCheck, TestStepResult, VerdictStatus,
    DEFAULT_RECEIPT_SCHEMA,
};
use std::path::PathBuf;

fn sample_passing_receipt() -> ExecutionReceipt {
    ExecutionReceipt {
        schema: DEFAULT_RECEIPT_SCHEMA.to_string(),
        task_id: "TICKET-06".to_string(),
        workspace: PathBuf::from("/workspace/k0maru-agent-verifier"),
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
                total: 42,
                failed: 0,
                duration_ms: Some(350),
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
                modified_files: vec![
                    "src/lib.rs".to_string(),
                    "tests/test_receipt.rs".to_string(),
                ],
                unauthorized_files: Vec::new(),
                message: None,
            },
            diff_coverage: None,
            red_check: None,
        },
        artifacts: ArtifactSummary {
            git_patch_sha256: Some(
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string(),
            ),
            receipt_timestamp: Some("2026-10-12T16:00:00Z".to_string()),
        },
    }
}

// ============================================================================
// 1. VerdictStatus and StepStatus Tests (methods, Display, serde case insensitivity)
// ============================================================================

#[test]
fn test_verdict_status_methods_and_display() {
    assert!(VerdictStatus::Success.is_success());
    assert!(!VerdictStatus::Success.is_failed());
    assert!(!VerdictStatus::Success.is_blocked());
    assert_eq!(VerdictStatus::Success.as_str(), "success");
    assert_eq!(VerdictStatus::Success.to_string(), "success");

    assert!(!VerdictStatus::Failed.is_success());
    assert!(VerdictStatus::Failed.is_failed());
    assert!(!VerdictStatus::Failed.is_blocked());
    assert_eq!(VerdictStatus::Failed.as_str(), "failed");
    assert_eq!(VerdictStatus::Failed.to_string(), "failed");

    assert!(!VerdictStatus::Blocked.is_success());
    assert!(!VerdictStatus::Blocked.is_failed());
    assert!(VerdictStatus::Blocked.is_blocked());
    assert_eq!(VerdictStatus::Blocked.as_str(), "blocked");
    assert_eq!(VerdictStatus::Blocked.to_string(), "blocked");
}

#[test]
fn test_verdict_status_case_insensitive_deserialization() {
    let cases = [
        ("\"success\"", VerdictStatus::Success),
        ("\"SUCCESS\"", VerdictStatus::Success),
        ("\"Success\"", VerdictStatus::Success),
        ("\"  SuCcEsS  \"", VerdictStatus::Success),
        ("\"failed\"", VerdictStatus::Failed),
        ("\"FAILED\"", VerdictStatus::Failed),
        ("\"Failed\"", VerdictStatus::Failed),
        ("\"  FaIlEd\\n\"", VerdictStatus::Failed),
        ("\"blocked\"", VerdictStatus::Blocked),
        ("\"BLOCKED\"", VerdictStatus::Blocked),
        ("\"Blocked\"", VerdictStatus::Blocked),
        ("\"  BLOCKED  \"", VerdictStatus::Blocked),
    ];

    for (json_str, expected) in cases {
        let parsed: VerdictStatus = serde_json::from_str(json_str)
            .unwrap_or_else(|e| panic!("Failed to parse {json_str}: {e}"));
        assert_eq!(parsed, expected, "Mismatch for input: {json_str}");
    }

    let invalid_cases = ["\"running\"", "\"pending\"", "\"error\"", "\"\"", "\"123\""];
    for invalid in invalid_cases {
        assert!(
            serde_json::from_str::<VerdictStatus>(invalid).is_err(),
            "Expected error for: {invalid}"
        );
    }
}

#[test]
fn test_step_status_methods_and_display() {
    assert!(StepStatus::Passed.is_passed());
    assert!(!StepStatus::Passed.is_failed());
    assert!(!StepStatus::Passed.is_skipped());
    assert_eq!(StepStatus::Passed.as_str(), "passed");
    assert_eq!(StepStatus::Passed.to_string(), "passed");

    assert!(!StepStatus::Failed.is_passed());
    assert!(StepStatus::Failed.is_failed());
    assert!(!StepStatus::Failed.is_skipped());
    assert_eq!(StepStatus::Failed.as_str(), "failed");
    assert_eq!(StepStatus::Failed.to_string(), "failed");

    assert!(!StepStatus::Skipped.is_passed());
    assert!(!StepStatus::Skipped.is_failed());
    assert!(StepStatus::Skipped.is_skipped());
    assert_eq!(StepStatus::Skipped.as_str(), "skipped");
    assert_eq!(StepStatus::Skipped.to_string(), "skipped");
}

#[test]
fn test_step_status_case_insensitive_deserialization() {
    let cases = [
        ("\"passed\"", StepStatus::Passed),
        ("\"PASSED\"", StepStatus::Passed),
        ("\"Passed\"", StepStatus::Passed),
        ("\"  PaSsEd  \"", StepStatus::Passed),
        ("\"failed\"", StepStatus::Failed),
        ("\"FAILED\"", StepStatus::Failed),
        ("\"Failed\"", StepStatus::Failed),
        ("\"  FAILED\\t\"", StepStatus::Failed),
        ("\"skipped\"", StepStatus::Skipped),
        ("\"SKIPPED\"", StepStatus::Skipped),
        ("\"Skipped\"", StepStatus::Skipped),
        ("\"  SKIPPED  \"", StepStatus::Skipped),
    ];

    for (json_str, expected) in cases {
        let parsed: StepStatus = serde_json::from_str(json_str)
            .unwrap_or_else(|e| panic!("Failed to parse {json_str}: {e}"));
        assert_eq!(parsed, expected, "Mismatch for input: {json_str}");
    }

    let invalid_cases = [
        "\"ok\"",
        "\"running\"",
        "\"timeout\"",
        "\"\"",
        "\"unknown\"",
    ];
    for invalid in invalid_cases {
        assert!(
            serde_json::from_str::<StepStatus>(invalid).is_err(),
            "Expected error for: {invalid}"
        );
    }
}

// ============================================================================
// 2. Individual Checks, Summary, and Backward Compatibility Aliases
// ============================================================================

#[test]
fn test_compilation_check_is_passed() {
    let passed = CompilationCheck {
        status: StepStatus::Passed,
        exit_code: 0,
        message: None,
    };
    assert!(passed.is_passed());

    let failed_exit = CompilationCheck {
        status: StepStatus::Passed,
        exit_code: 1,
        message: None,
    };
    assert!(!failed_exit.is_passed());

    let failed_status = CompilationCheck {
        status: StepStatus::Failed,
        exit_code: 0,
        message: Some("compiler warning treated as error".to_string()),
    };
    assert!(!failed_status.is_passed());

    let skipped = CompilationCheck {
        status: StepStatus::Skipped,
        exit_code: 0,
        message: None,
    };
    assert!(!skipped.is_passed());
}

#[test]
fn test_test_check_is_passed() {
    let passed = TestCheck {
        status: StepStatus::Passed,
        exit_code: Some(0),
        total: 10,
        failed: 0,
        duration_ms: Some(120),
        message: None,
        flaky_quarantined: None,
        retries: None,
    };
    assert!(passed.is_passed());

    let passed_no_exit_code = TestCheck {
        status: StepStatus::Passed,
        exit_code: None, // defaults to 0
        total: 5,
        failed: 0,
        duration_ms: None,
        message: None,
        flaky_quarantined: None,
        retries: None,
    };
    assert!(passed_no_exit_code.is_passed());

    let failed_tests = TestCheck {
        status: StepStatus::Passed,
        exit_code: Some(0),
        total: 10,
        failed: 1,
        duration_ms: None,
        message: None,
        flaky_quarantined: None,
        retries: None,
    };
    assert!(!failed_tests.is_passed());

    let non_zero_exit = TestCheck {
        status: StepStatus::Passed,
        exit_code: Some(101),
        total: 10,
        failed: 0,
        duration_ms: None,
        message: None,
        flaky_quarantined: None,
        retries: None,
    };
    assert!(!non_zero_exit.is_passed());

    let failed_status = TestCheck {
        status: StepStatus::Failed,
        exit_code: Some(1),
        total: 1,
        failed: 1,
        duration_ms: None,
        message: None,
        flaky_quarantined: None,
        retries: None,
    };
    assert!(!failed_status.is_passed());
}

#[test]
fn test_linter_check_is_passed() {
    let passed = LinterCheck {
        status: StepStatus::Passed,
        warnings: 0,
        exit_code: Some(0),
        message: None,
    };
    assert!(passed.is_passed());

    let has_warnings = LinterCheck {
        status: StepStatus::Passed,
        warnings: 2,
        exit_code: Some(0),
        message: None,
    };
    assert!(!has_warnings.is_passed());

    let failed_exit = LinterCheck {
        status: StepStatus::Passed,
        warnings: 0,
        exit_code: Some(1),
        message: None,
    };
    assert!(!failed_exit.is_passed());
}

#[test]
fn test_scope_guard_check_is_passed() {
    let passed = ScopeGuardCheck {
        status: StepStatus::Passed,
        modified_files: vec!["src/lib.rs".to_string()],
        unauthorized_files: Vec::new(),
        message: None,
    };
    assert!(passed.is_passed());

    let unauthorized = ScopeGuardCheck {
        status: StepStatus::Passed,
        modified_files: vec!["src/lib.rs".to_string(), "bad.key".to_string()],
        unauthorized_files: vec!["bad.key".to_string()],
        message: None,
    };
    assert!(!unauthorized.is_passed());

    let skipped = ScopeGuardCheck {
        status: StepStatus::Skipped,
        modified_files: Vec::new(),
        unauthorized_files: Vec::new(),
        message: None,
    };
    assert!(!skipped.is_passed());
}

#[test]
fn test_check_summary_is_all_passed() {
    let receipt = sample_passing_receipt();
    assert!(receipt.checks.is_all_passed());

    // Fails if compilation fails
    let mut c1 = receipt.checks.clone();
    c1.compilation.status = StepStatus::Failed;
    assert!(!c1.is_all_passed());

    // Fails if tests fail
    let mut c2 = receipt.checks.clone();
    c2.tests.failed = 1;
    assert!(!c2.is_all_passed());

    // Fails if linter has warnings
    let mut c3 = receipt.checks.clone();
    c3.linter.warnings = 1;
    assert!(!c3.is_all_passed());

    // Fails if scope guard has unauthorized files
    let mut c4 = receipt.checks.clone();
    c4.scope_guard.unauthorized_files = vec!["unauthorized.txt".to_string()];
    assert!(!c4.is_all_passed());

    // Compatibility aliases
    let alias_step: StepResult = c1.compilation;
    assert!(!alias_step.is_passed());
    let alias_test: TestStepResult = c2.tests;
    assert!(!alias_test.is_passed());
}

// ============================================================================
// 3. JSON Roundtripping & Schema Conformance
// ============================================================================

#[test]
fn test_json_roundtrip_conforms_to_schema() {
    let receipt = sample_passing_receipt();
    let json_string = serde_json::to_string_pretty(&receipt).expect("serialize receipt");

    // Parse as raw serde_json::Value to check field existence conforming to schema
    let val: serde_json::Value = serde_json::from_str(&json_string).expect("parse JSON value");

    // Top-level schema properties
    assert_eq!(
        val["$schema"].as_str().unwrap(),
        "https://k0maru.dev/schemas/agent-receipt-v1.json"
    );
    assert_eq!(val["task_id"].as_str().unwrap(), "TICKET-06");
    assert_eq!(
        val["workspace"].as_str().unwrap(),
        "/workspace/k0maru-agent-verifier"
    );
    assert_eq!(val["verdict"].as_str().unwrap(), "success");

    // Checks properties
    let checks = &val["checks"];
    assert!(checks["compilation"].is_object());
    assert_eq!(checks["compilation"]["status"], "passed");
    assert_eq!(checks["compilation"]["exit_code"], 0);

    assert!(checks["tests"].is_object());
    assert_eq!(checks["tests"]["status"], "passed");
    assert_eq!(checks["tests"]["total"], 42);
    assert_eq!(checks["tests"]["failed"], 0);
    assert_eq!(checks["tests"]["duration_ms"], 350);

    assert!(checks["linter"].is_object());
    assert_eq!(checks["linter"]["status"], "passed");
    assert_eq!(checks["linter"]["warnings"], 0);

    assert!(checks["scope_guard"].is_object());
    assert_eq!(checks["scope_guard"]["status"], "passed");
    assert!(checks["scope_guard"]["modified_files"].is_array());

    // Artifacts properties
    let artifacts = &val["artifacts"];
    assert_eq!(
        artifacts["git_patch_sha256"].as_str().unwrap(),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        artifacts["receipt_timestamp"].as_str().unwrap(),
        "2026-10-12T16:00:00Z"
    );

    // Deserialize back into ExecutionReceipt and check equality
    let deserialized: ExecutionReceipt =
        serde_json::from_str(&json_string).expect("deserialize receipt");
    assert_eq!(receipt, deserialized);
    assert!(deserialized.is_passing());
    assert!(deserialized.validate().is_ok());
}

#[test]
fn test_deserialize_minimal_agent_receipt_schema() {
    let json_text = r#"
{
  "$schema": "https://k0maru.dev/schemas/agent-receipt-v1.json",
  "task_id": "minimal-01",
  "workspace": "/tmp/minimal",
  "verdict": "success",
  "checks": {
    "compilation": {
      "status": "passed",
      "exit_code": 0
    },
    "tests": {
      "status": "passed",
      "total": 0,
      "failed": 0
    },
    "linter": {
      "status": "passed",
      "warnings": 0
    },
    "scope_guard": {
      "status": "passed",
      "modified_files": []
    }
  },
  "artifacts": {}
}
"#;

    let receipt: ExecutionReceipt = serde_json::from_str(json_text).expect("deserialization");
    assert_eq!(receipt.task_id, "minimal-01");
    assert_eq!(receipt.workspace, PathBuf::from("/tmp/minimal"));
    assert_eq!(receipt.verdict, VerdictStatus::Success);
    assert!(receipt.is_passing());
    assert!(receipt.artifacts.git_patch_sha256.is_none());
    assert!(receipt.artifacts.receipt_timestamp.is_none());
    assert!(receipt.validate().is_ok());
}

// ============================================================================
// 4. SHA-256 Calculation and Verification Tests
// ============================================================================

#[test]
fn test_compute_sha256_known_vectors() {
    // SHA-256 of empty bytes
    let empty_hash = ExecutionReceipt::compute_sha256(b"");
    assert_eq!(
        empty_hash,
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );

    // SHA-256 of "hello world\n"
    let hw_hash = ExecutionReceipt::compute_sha256(b"hello world\n");
    assert_eq!(
        hw_hash,
        "a948904f2f0f479b8f8197694b30184b0d2ed1c1cd2a1ec0fb85d299a192a447"
    );

    // Large payload
    let large_data = vec![0xABu8; 100_000];
    let large_hash = ExecutionReceipt::compute_sha256(&large_data);
    assert_eq!(large_hash.len(), 64);
    assert!(large_hash.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn test_verify_patch_sha256_matching_and_mismatch() {
    let mut receipt = sample_passing_receipt();
    let patch = b"diff --git a/foo.rs b/foo.rs\n+pub fn test() {}\n";
    let hash = ExecutionReceipt::compute_sha256(patch);

    receipt.artifacts.git_patch_sha256 = Some(hash.clone());

    // Exact match
    assert!(receipt.verify_patch_sha256(patch).unwrap());

    // Mismatched content
    assert!(!receipt.verify_patch_sha256(b"different patch").unwrap());

    // Case insensitivity of hex hash in artifact
    receipt.artifacts.git_patch_sha256 = Some(hash.to_uppercase());
    assert!(receipt.verify_patch_sha256(patch).unwrap());

    // None in artifacts always returns Ok(true)
    receipt.artifacts.git_patch_sha256 = None;
    assert!(receipt.verify_patch_sha256(patch).unwrap());
    assert!(receipt.verify_patch_sha256(b"arbitrary bytes").unwrap());
}

#[test]
fn test_verify_patch_sha256_malformed_formats() {
    let mut receipt = sample_passing_receipt();

    // Too short
    receipt.artifacts.git_patch_sha256 = Some("abcd".to_string());
    assert!(matches!(
        receipt.verify_patch_sha256(b"foo"),
        Err(VerifierError::InvalidReceipt(_))
    ));

    // Non-hex characters
    receipt.artifacts.git_patch_sha256 =
        Some("z3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string());
    assert!(matches!(
        receipt.verify_patch_sha256(b"foo"),
        Err(VerifierError::InvalidReceipt(_))
    ));
}

// ============================================================================
// 5. Receipt Invariant Validation Edge Cases (validate())
// ============================================================================

#[test]
fn test_validate_task_id_edge_cases() {
    let mut r = sample_passing_receipt();

    // Empty task_id
    r.task_id = "".to_string();
    assert!(matches!(
        r.validate(),
        Err(VerifierError::InvalidReceipt(_))
    ));

    r.task_id = "   \t".to_string();
    assert!(matches!(
        r.validate(),
        Err(VerifierError::InvalidReceipt(_))
    ));

    // Path traversal in task_id
    r.task_id = "../evil".to_string();
    assert!(matches!(
        r.validate(),
        Err(VerifierError::InvalidReceipt(_))
    ));

    r.task_id = "foo/../bar".to_string();
    assert!(matches!(
        r.validate(),
        Err(VerifierError::InvalidReceipt(_))
    ));

    r.task_id = "sub/task".to_string();
    assert!(matches!(
        r.validate(),
        Err(VerifierError::InvalidReceipt(_))
    ));

    r.task_id = "sub\\task".to_string();
    assert!(matches!(
        r.validate(),
        Err(VerifierError::InvalidReceipt(_))
    ));

    r.task_id = "null\0byte".to_string();
    assert!(matches!(
        r.validate(),
        Err(VerifierError::InvalidReceipt(_))
    ));

    // Valid task IDs
    r.task_id = "ticket-06_part-A.123".to_string();
    assert!(r.validate().is_ok());
}

#[test]
fn test_validate_workspace_edge_cases() {
    let mut r = sample_passing_receipt();
    r.workspace = PathBuf::from("");
    assert!(matches!(
        r.validate(),
        Err(VerifierError::InvalidReceipt(_))
    ));
}

#[test]
fn test_validate_compilation_mismatch() {
    let mut r = sample_passing_receipt();
    r.checks.compilation.status = StepStatus::Passed;
    r.checks.compilation.exit_code = 1;
    let err = r.validate().expect_err("should fail");
    assert!(err
        .to_string()
        .contains("Compilation marked 'passed' but exit_code is non-zero"));

    r.checks.compilation.exit_code = -1;
    assert!(r.validate().is_err());
}

#[test]
fn test_validate_tests_mismatch() {
    let mut r = sample_passing_receipt();
    // Tests marked passed but failed > 0
    r.checks.tests.status = StepStatus::Passed;
    r.checks.tests.failed = 1;
    let err = r.validate().expect_err("should fail");
    assert!(err
        .to_string()
        .contains("Tests marked 'passed' but failed count is non-zero"));

    // Tests failed > total
    r.checks.tests.status = StepStatus::Failed;
    r.verdict = VerdictStatus::Failed;
    r.checks.tests.total = 5;
    r.checks.tests.failed = 10;
    let err2 = r.validate().expect_err("should fail");
    assert!(err2
        .to_string()
        .contains("Tests failed count (10) exceeds total test count (5)"));
}

#[test]
fn test_validate_linter_mismatch() {
    let mut r = sample_passing_receipt();
    r.checks.linter.status = StepStatus::Passed;
    r.checks.linter.warnings = 4;
    let err = r.validate().expect_err("should fail");
    assert!(err
        .to_string()
        .contains("Linter marked 'passed' but warnings count is non-zero"));
}

#[test]
fn test_validate_scope_guard_mismatch() {
    let mut r = sample_passing_receipt();
    r.checks.scope_guard.status = StepStatus::Passed;
    r.checks.scope_guard.unauthorized_files = vec!["leaked.key".to_string()];
    let err = r.validate().expect_err("should fail");
    assert!(err
        .to_string()
        .contains("Scope guard marked 'passed' but unauthorized_files is non-empty"));
}

#[test]
fn test_validate_verdict_consistency() {
    let mut r = sample_passing_receipt();
    // Verdict is Success, but checks has failure
    r.verdict = VerdictStatus::Success;
    r.checks.compilation.status = StepStatus::Failed;
    r.checks.compilation.exit_code = 1;
    let err = r.validate().expect_err("should fail");
    assert!(err
        .to_string()
        .contains("Verdict is marked 'success' but not all checks passed"));

    // Verdict is Failed or Blocked with failing checks is valid
    r.verdict = VerdictStatus::Failed;
    assert!(r.validate().is_ok());

    r.verdict = VerdictStatus::Blocked;
    assert!(r.validate().is_ok());
}

#[test]
fn test_validate_timestamp_formats() {
    let mut r = sample_passing_receipt();

    // Valid RFC-3339 timestamps
    r.artifacts.receipt_timestamp = Some("2026-10-12T16:00:00Z".to_string());
    assert!(r.validate().is_ok());

    r.artifacts.receipt_timestamp = Some("2026-10-12T18:30:45.123+08:00".to_string());
    assert!(r.validate().is_ok());

    // Invalid timestamp strings
    r.artifacts.receipt_timestamp = Some("not-rfc3339".to_string());
    assert!(matches!(
        r.validate(),
        Err(VerifierError::InvalidReceipt(_))
    ));

    r.artifacts.receipt_timestamp = Some("2026/10/12 16:00:00".to_string());
    assert!(matches!(
        r.validate(),
        Err(VerifierError::InvalidReceipt(_))
    ));
}

#[test]
fn test_validate_git_patch_sha256_format() {
    let mut r = sample_passing_receipt();

    // Valid 64-char hex
    r.artifacts.git_patch_sha256 =
        Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string());
    assert!(r.validate().is_ok());

    // Invalid length
    r.artifacts.git_patch_sha256 = Some("e3b0c442".to_string());
    assert!(matches!(
        r.validate(),
        Err(VerifierError::InvalidReceipt(_))
    ));

    // Non-hex
    r.artifacts.git_patch_sha256 =
        Some("g3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string());
    assert!(matches!(
        r.validate(),
        Err(VerifierError::InvalidReceipt(_))
    ));
}

// ============================================================================
// 6. Summary and Display Formatting Tests
// ============================================================================

#[test]
fn test_summary_and_display_formatting() {
    let mut receipt = sample_passing_receipt();
    receipt.checks.compilation.message = Some("Compiled cleanly with no errors".to_string());

    let summary = receipt.summary();
    assert!(summary.contains("Receipt [TICKET-06 | PASSING]"));
    assert!(summary.contains("Workspace: /workspace/k0maru-agent-verifier"));
    assert!(summary.contains("Verdict: success"));
    assert!(summary.contains("compilation: passed (exit 0) - message: Compiled cleanly"));
    assert!(summary.contains("tests: passed (0/42 failed, duration: 350ms)"));
    assert!(summary.contains("linter: passed (0 warnings)"));
    assert!(summary.contains("scope_guard: passed (2 modified, 0 unauthorized)"));
    assert!(summary.contains(
        "patch sha256: e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    ));
    assert!(summary.contains("timestamp: 2026-10-12T16:00:00Z"));

    // Display implementation matches summary()
    assert_eq!(format!("{receipt}"), summary);

    // Failing summary formatting
    receipt.verdict = VerdictStatus::Failed;
    receipt.checks.tests.status = StepStatus::Failed;
    receipt.checks.tests.failed = 3;
    receipt.artifacts = ArtifactSummary::default();

    let fail_summary = receipt.summary();
    assert!(fail_summary.contains("Receipt [TICKET-06 | FAILING]"));
    assert!(fail_summary.contains("tests: failed (3/42 failed"));
    assert!(fail_summary.contains("patch sha256: <none>"));
    assert!(fail_summary.contains("timestamp: <none>"));
}

#[test]
fn test_resolve_receipts_dir_hierarchy() {
    use k0maru_verifier::receipt::resolve_receipts_dir;
    use std::path::{Path, PathBuf};

    // 1. Explicit override directory takes highest priority
    let custom = Path::new("/custom/receipts/dir");
    assert_eq!(
        resolve_receipts_dir(Some(custom)),
        PathBuf::from("/custom/receipts/dir")
    );

    // 2. K0MARU_RECEIPT_DIR env var takes second priority
    std::env::set_var("K0MARU_RECEIPT_DIR", "/env/receipts/dir");
    assert_eq!(
        resolve_receipts_dir(None),
        PathBuf::from("/env/receipts/dir")
    );

    // Empty env var should fall through to default
    std::env::set_var("K0MARU_RECEIPT_DIR", "   ");
    let fallback = resolve_receipts_dir(None);
    assert!(fallback.ends_with(".k0maru/receipts") || fallback.ends_with(".k0maru\\receipts"));

    // Clean up env
    std::env::remove_var("K0MARU_RECEIPT_DIR");
    let default_dir = resolve_receipts_dir(None);
    assert!(
        default_dir.ends_with(".k0maru/receipts") || default_dir.ends_with(".k0maru\\receipts")
    );
}
