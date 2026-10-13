use assert_cmd::Command;
use k0maru_verifier::kdsl::KdslVerifyClause;
use k0maru_verifier::tuple::LindaTuple;
use predicates::prelude::*;
use tempfile::TempDir;

fn setup_temp_cargo_workspace_with_git() -> TempDir {
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
    run_git(&["config", "user.name", "KdslTester"]);
    run_git(&["config", "user.email", "tester@kdsl.com"]);
    std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "dummy_kdsl"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/lib.rs"), "pub fn ok() -> bool { true }\n").unwrap();
    std::fs::create_dir_all(root.join("tests")).unwrap();
    std::fs::write(
        root.join("tests/baseline.rs"),
        "#[test]\nfn test_base() {\n    let val = dummy_kdsl::ok();\n    assert!(val);\n}\n",
    )
    .unwrap();
    run_git(&["add", "."]);
    run_git(&["commit", "-m", "init baseline"]);
    temp
}

#[test]
fn test_kdsl_parser_invariants_and_targets() {
    let clause = KdslVerifyClause::parse(
        "!Σ:VERIFY(COMPILE, TEST, SCOPE, LIVING_ADR#SEC-004, IMMUTABLE_TESTS#main)",
    )
    .expect("parse verify clause");

    assert!(clause.has_compile());
    assert!(clause.has_test());
    assert!(clause.has_scope());
    assert!(clause.has_living_adr());
    assert_eq!(clause.living_adr_target(), Some("SEC-004"));
    assert!(clause.has_immutable_tests());
    assert_eq!(clause.immutable_base_branch(), Some("main"));
}

#[test]
fn test_kdsl_parser_wire_line_domain_extraction() {
    let wire = "!δ:0.1|AUTH-GATEWAY|src/auth.ts#v10 §AST:F[verifyToken] @OP:SWAP (a) -> (b) !Σ:VERIFY(CARGO_CHECK, LIVING_ADR#SEC-009)";
    let clause = KdslVerifyClause::parse(wire).expect("parse wire");

    assert_eq!(clause.domain.as_deref(), Some("AUTH-GATEWAY"));
    assert!(clause.has_compile());
    assert_eq!(clause.living_adr_target(), Some("SEC-009"));
}

#[test]
fn test_cli_kdsl_wire_sets_domain_for_tuple_emission() {
    let ws = setup_temp_cargo_workspace_with_git();
    let temp_tuples = TempDir::new().unwrap();

    let kdsl_wire =
        "!δ:0.1|SEC-CORE|src/lib.rs#v1 §AST:F[ok] @OP:NOOP !Σ:VERIFY(COMPILE,TEST,SCOPE)";

    let mut cmd = Command::cargo_bin("k0maru-verifier").unwrap();
    cmd.args([
        "verify",
        "--workspace",
        ws.path().to_str().unwrap(),
        "--task-id",
        "task-kdsl-domain-01",
        "--kdsl",
        kdsl_wire,
        "--tuple-dir",
        temp_tuples.path().to_str().unwrap(),
    ])
    .assert()
    .success();

    let entries: Vec<_> = std::fs::read_dir(temp_tuples.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(entries.len(), 1);

    let content = std::fs::read_to_string(entries[0].path()).unwrap();
    let tuple: LindaTuple = serde_json::from_str(&content).expect("parse emitted tuple");
    assert_eq!(
        tuple.domain, "SEC-CORE",
        "Domain should be automatically extracted from K-DSL wire header"
    );
    assert_eq!(tuple.kind, "TASK_VERIFIED");
}

#[test]
fn test_cli_kdsl_immutable_tests_blocks_tampering() {
    let ws = setup_temp_cargo_workspace_with_git();
    let root = ws.path();

    let run_git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .expect("git execution");
        assert!(output.status.success());
    };

    // Checkout feature branch and delete line from baseline test
    run_git(&["checkout", "-b", "feat/tamper"]);
    std::fs::write(
        root.join("tests/baseline.rs"),
        "#[test]\nfn test_base() {\n    assert!(true);\n}\n",
    )
    .unwrap();

    let kdsl_wire = "!Σ:VERIFY(COMPILE, TEST, IMMUTABLE_TESTS#main)";
    let mut cmd = Command::cargo_bin("k0maru-verifier").unwrap();
    cmd.args([
        "verify",
        "--workspace",
        root.to_str().unwrap(),
        "--task-id",
        "task-kdsl-tamper-check",
        "--kdsl",
        kdsl_wire,
    ])
    .assert()
    .code(1)
    .stdout(predicate::str::contains(
        "Anti-tampering violation: deletions detected in existing test baseline",
    ));
}
