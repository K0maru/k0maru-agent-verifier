use k0maru_verifier::runner::{
    count_clippy_warnings, parse_cargo_test_output, parse_pytest_output, truncate_output,
    CargoRunner, LanguageRunner, NpmRunner, PytestRunner, RunnerRegistry,
};
use tempfile::TempDir;

// ============================================================================
// 1. Language Runner Detection Tests (Synthetic Fixtures)
// ============================================================================

#[test]
fn test_cargo_runner_detection() {
    let runner = CargoRunner;
    assert_eq!(runner.name(), "cargo");

    let temp = TempDir::new().unwrap();
    // Empty workspace -> not detected
    assert!(!runner.detect(temp.path()));

    // Cargo.toml exists -> detected
    std::fs::write(
        temp.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    assert!(runner.detect(temp.path()));
}

#[test]
fn test_pytest_runner_detection_various_markers() {
    let runner = PytestRunner;
    assert_eq!(runner.name(), "pytest");

    let markers = [
        "pyproject.toml",
        "setup.py",
        "requirements.txt",
        "pytest.ini",
        "tox.ini",
        "setup.cfg",
    ];

    for marker in markers {
        let temp = TempDir::new().unwrap();
        assert!(
            !runner.detect(temp.path()),
            "Empty directory should not be detected"
        );

        std::fs::write(temp.path().join(marker), "# python marker").unwrap();
        assert!(
            runner.detect(temp.path()),
            "Directory with {marker} should be detected by PytestRunner"
        );
    }

    // Unrelated file
    let temp_unrelated = TempDir::new().unwrap();
    std::fs::write(temp_unrelated.path().join("README.md"), "# Python docs").unwrap();
    assert!(!runner.detect(temp_unrelated.path()));
}

#[test]
fn test_npm_runner_detection() {
    let runner = NpmRunner;
    assert_eq!(runner.name(), "npm");

    let temp = TempDir::new().unwrap();
    assert!(!runner.detect(temp.path()));

    std::fs::write(
        temp.path().join("package.json"),
        r#"{"name": "test-app", "version": "1.0.0"}"#,
    )
    .unwrap();
    assert!(runner.detect(temp.path()));
}

// ============================================================================
// 2. Output Parsing Tests (cargo test, pytest, clippy)
// ============================================================================

#[test]
fn test_parse_cargo_test_output_single_passing() {
    let stdout = r#"
running 4 tests
test test_a ... ok
test test_b ... ok
test test_c ... ok
test test_d ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
"#;
    let (total, failed) = parse_cargo_test_output(stdout).expect("parse passed");
    assert_eq!(total, 4);
    assert_eq!(failed, 0);
}

#[test]
fn test_parse_cargo_test_output_single_failing() {
    let stdout = r#"
running 5 tests
test test_a ... ok
test test_b ... FAILED
test test_c ... ok
test test_d ... FAILED
test test_e ... ok

failures:
    test_b
    test_d

test result: FAILED. 3 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s
"#;
    let (total, failed) = parse_cargo_test_output(stdout).expect("parse failed");
    assert_eq!(total, 5);
    assert_eq!(failed, 2);
}

#[test]
fn test_parse_cargo_test_output_multiple_test_suites() {
    let stdout = r#"
running 3 tests
test test_unit_1 ... ok
test test_unit_2 ... ok
test test_unit_3 ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s

     Running tests/integration_test.rs (target/debug/deps/integration_test-123456)

running 2 tests
test test_integ_1 ... ok
test test_integ_2 ... FAILED

test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s

   Doc-tests k0maru_verifier

running 1 test
test src/lib.rs - (line 10) ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
"#;
    let (total, failed) = parse_cargo_test_output(stdout).expect("parse multiple suites");
    // 3 + (1 + 1) + 1 = 6 total, 1 failed
    assert_eq!(total, 6);
    assert_eq!(failed, 1);
}

#[test]
fn test_parse_cargo_test_output_zero_tests_and_unmatched() {
    let zero_stdout =
        "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s";
    let (total, failed) = parse_cargo_test_output(zero_stdout).expect("parse zero tests");
    assert_eq!(total, 0);
    assert_eq!(failed, 0);

    // Empty or arbitrary output
    assert!(parse_cargo_test_output("").is_none());
    assert!(parse_cargo_test_output("Compiling demo v0.1.0\nFinished dev target").is_none());
    assert!(parse_cargo_test_output("error: test failed, to rerun pass '--bin demo'").is_none());
}

#[test]
fn test_parse_pytest_output_variations() {
    // Only passed
    let pass_only = "================ 15 passed in 0.42s ================";
    let (t1, f1) = parse_pytest_output(pass_only).expect("parse pass only");
    assert_eq!(t1, 15);
    assert_eq!(f1, 0);

    // Passed and failed
    let pass_and_fail = "================ 12 passed, 3 failed in 1.15s ================";
    let (t2, f2) = parse_pytest_output(pass_and_fail).expect("parse pass and fail");
    assert_eq!(t2, 15);
    assert_eq!(f2, 3);

    // Passed, failed, and errors
    let pass_fail_errors = "================ 8 passed, 2 failed, 1 error in 2.05s ================";
    let (t3, f3) = parse_pytest_output(pass_fail_errors).expect("parse with error");
    assert_eq!(t3, 11);
    assert_eq!(f3, 3);

    // Plural errors
    let pass_fail_plural_errors =
        "================ 10 passed, 1 failed, 2 errors in 0.88s ================";
    let (t4, f4) = parse_pytest_output(pass_fail_plural_errors).expect("parse plural errors");
    assert_eq!(t4, 13);
    assert_eq!(f4, 3);

    // Only failed
    let fail_only = "================ 4 failed in 0.12s ================";
    let (t5, f5) = parse_pytest_output(fail_only).expect("parse fail only");
    assert_eq!(t5, 4);
    assert_eq!(f5, 4);

    // Unrelated text
    assert!(parse_pytest_output("").is_none());
    assert!(parse_pytest_output("pytest: command not found").is_none());
}

#[test]
fn test_count_clippy_warnings_filtering() {
    let clippy_output = r#"
warning: field `unused` is never read
  --> src/types.rs:12:5
   |
12 |     unused: bool,
   |     ^^^^^^

warning: variable does not need to be mutable
  --> src/main.rs:20:9
   |
20 |     let mut x = 42;
   |         ----^

error: aborting due to 2 previous errors
error: could not compile `k0maru-verifier` (bin "k0maru-verifier") due to 1 previous error
"#;

    let count = count_clippy_warnings(clippy_output);
    // 2 warnings; the 2 compiler diagnostic summary error lines should be ignored
    assert_eq!(count, 2);

    // Real linter error (e.g. -D warnings elevation)
    let elevated_error = r#"
error: unused variable: `foo`
  --> src/lib.rs:5:9
   |
5  |     let foo = 1;
   |         ^^^
   |
   = note: `-D unused-variables` implied by `-D warnings`

error: could not compile `demo` due to previous error
"#;
    let count2 = count_clippy_warnings(elevated_error);
    assert_eq!(count2, 1);

    // Clean output
    assert_eq!(count_clippy_warnings(""), 0);
    assert_eq!(count_clippy_warnings("Finished `dev` profile"), 0);
}

// ============================================================================
// 3. RunnerRegistry Tests (Priority order, registration, lookup)
// ============================================================================

#[test]
fn test_runner_registry_defaults_and_priority() {
    let registry = RunnerRegistry::new();
    assert_eq!(registry.runner_names(), vec!["cargo", "pytest", "npm"]);
    assert_eq!(registry.runners().len(), 3);

    // Workspace containing both Cargo.toml and package.json
    let multi_temp = TempDir::new().unwrap();
    std::fs::write(multi_temp.path().join("Cargo.toml"), "[package]").unwrap();
    std::fs::write(multi_temp.path().join("package.json"), "{}").unwrap();

    // Default detection order must pick cargo first
    let detected = registry
        .detect_runner(multi_temp.path())
        .expect("should detect runner");
    assert_eq!(detected.name(), "cargo");

    // Workspace with only pytest marker
    let py_temp = TempDir::new().unwrap();
    std::fs::write(py_temp.path().join("setup.py"), "# setup").unwrap();
    let detected_py = registry
        .detect_runner(py_temp.path())
        .expect("should detect pytest");
    assert_eq!(detected_py.name(), "pytest");

    // Workspace with only npm marker
    let npm_temp = TempDir::new().unwrap();
    std::fs::write(npm_temp.path().join("package.json"), "{}").unwrap();
    let detected_npm = registry
        .detect_runner(npm_temp.path())
        .expect("should detect npm");
    assert_eq!(detected_npm.name(), "npm");

    // Workspace with no markers
    let empty_temp = TempDir::new().unwrap();
    assert!(registry.detect_runner(empty_temp.path()).is_none());
}

#[test]
fn test_runner_registry_name_lookup() {
    let registry = RunnerRegistry::new();

    // Case-insensitivity and whitespace trimming
    assert!(registry.get_by_name("cargo").is_some());
    assert!(registry.get_by_name("CARGO").is_some());
    assert!(registry.get_by_name("  Cargo  ").is_some());

    assert!(registry.get_by_name("pytest").is_some());
    assert!(registry.get_by_name("PYTEST").is_some());

    assert!(registry.get_by_name("npm").is_some());
    assert!(registry.get_by_name("Npm").is_some());

    assert!(registry.get_by_name("non_existent").is_none());
    assert!(registry.get_by_name("").is_none());
}

#[test]
fn test_runner_registry_custom_registration_and_priority_override() {
    let mut registry = RunnerRegistry::empty();
    assert!(registry.runners().is_empty());
    assert!(registry.runner_names().is_empty());

    // Register at back
    registry.register(Box::new(NpmRunner));
    assert_eq!(registry.runner_names(), vec!["npm"]);

    // Register at front (highest priority)
    registry.register_front(Box::new(PytestRunner));
    assert_eq!(registry.runner_names(), vec!["pytest", "npm"]);

    // In a directory with both pyproject.toml and package.json, pytest should now win
    let temp = TempDir::new().unwrap();
    std::fs::write(temp.path().join("pyproject.toml"), "").unwrap();
    std::fs::write(temp.path().join("package.json"), "{}").unwrap();

    let detected = registry.detect_runner(temp.path()).unwrap();
    assert_eq!(detected.name(), "pytest");
}

// ============================================================================
// 4. Output Truncation Tests (truncate_output)
// ============================================================================

#[test]
fn test_truncate_output_ascii_boundaries() {
    // Shorter than max_bytes
    let input = b"Hello, Rust!";
    assert_eq!(truncate_output(input, 50), "Hello, Rust!");

    // Exactly equal to max_bytes
    assert_eq!(truncate_output(input, input.len()), "Hello, Rust!");

    // Longer than max_bytes
    let long_input = b"0123456789abcdefghij"; // 20 bytes
    let truncated = truncate_output(long_input, 10);
    assert!(truncated.starts_with("0123456789"));
    assert!(truncated.ends_with("... [truncated, total 20 bytes]"));

    // max_bytes == 0
    let zero_trunc = truncate_output(long_input, 0);
    assert_eq!(zero_trunc, "\n... [truncated, total 20 bytes]");

    // Empty input
    assert_eq!(truncate_output(b"", 10), "");
    assert_eq!(truncate_output(b"", 0), "");
}

#[test]
fn test_truncate_output_utf8_multibyte_safety() {
    // 1. 2-byte UTF-8: Greek "αβγδε" (2 bytes per char, total 10 bytes)
    let greek = "αβγδε";
    let greek_bytes = greek.as_bytes();
    assert_eq!(greek_bytes.len(), 10);

    // Truncating at 3 bytes falls in the middle of 'β' (bytes 2..4)
    // It should cleanly back up to byte 2 ("α")
    let trunc_greek = truncate_output(greek_bytes, 3);
    assert!(trunc_greek.starts_with("α\n..."));
    assert!(trunc_greek.contains("total 10 bytes"));

    // 2. 3-byte UTF-8: Japanese "こんにちは" (3 bytes per char, total 15 bytes)
    let jp = "こんにちは";
    let jp_bytes = jp.as_bytes();
    assert_eq!(jp_bytes.len(), 15);

    // Truncating at 5 bytes falls in the middle of 'ん' (bytes 3..6)
    // Should back up to 3 bytes ("こ")
    let trunc_jp = truncate_output(jp_bytes, 5);
    assert!(trunc_jp.starts_with("こ\n..."));

    // Truncating at 6 bytes is an exact boundary for "こん"
    let trunc_jp_exact = truncate_output(jp_bytes, 6);
    assert!(trunc_jp_exact.starts_with("こん\n..."));

    // 3. 4-byte UTF-8: Emojis "🚀🦀🎉" (4 bytes per emoji, total 12 bytes)
    let emoji = "🚀🦀🎉";
    let emoji_bytes = emoji.as_bytes();
    assert_eq!(emoji_bytes.len(), 12);

    // Truncating at 6 bytes falls in the middle of '🦀' (bytes 4..8)
    // Should back up to 4 bytes ("🚀")
    let trunc_emoji = truncate_output(emoji_bytes, 6);
    assert!(trunc_emoji.starts_with("🚀\n..."));
    assert!(trunc_emoji.contains("total 12 bytes"));

    // Truncating at 1 byte on multibyte sequence should back up to 0 bytes without panic
    let trunc_at_1 = truncate_output(emoji_bytes, 1);
    assert_eq!(trunc_at_1, "\n... [truncated, total 12 bytes]");
}
