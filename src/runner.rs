use crate::error::VerifierError;
use crate::receipt::{CompilationCheck, LinterCheck, StepStatus, TestCheck};
use async_trait::async_trait;
use regex::Regex;
use std::path::Path;
use std::time::Instant;
use tokio::process::Command;

/// Default maximum bytes to capture from command output before truncation (64 KB).
pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 64 * 1024;

/// Result of executing an external physical process.
#[derive(Debug, Clone)]
pub struct ExecutedCommand {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
}

/// Safely truncates byte output to `max_bytes` without splitting UTF-8 code points.
pub fn truncate_output(bytes: &[u8], max_bytes: usize) -> String {
    if bytes.len() <= max_bytes {
        String::from_utf8_lossy(bytes).into_owned()
    } else {
        let mut end = max_bytes;
        while end > 0 && !is_char_boundary(bytes, end) {
            end -= 1;
        }
        let truncated = String::from_utf8_lossy(&bytes[..end]);
        format!(
            "{}\n... [truncated, total {} bytes]",
            truncated,
            bytes.len()
        )
    }
}

fn is_char_boundary(bytes: &[u8], index: usize) -> bool {
    if index == 0 || index >= bytes.len() {
        true
    } else {
        (bytes[index] & 0xC0) != 0x80
    }
}

/// Executes a shell process with output capture and truncation safety.
pub async fn run_command(
    program: &str,
    args: &[&str],
    workspace: &Path,
    max_output_bytes: usize,
) -> Result<ExecutedCommand, VerifierError> {
    let start = Instant::now();
    let res = Command::new(program)
        .args(args)
        .current_dir(workspace)
        .output()
        .await;

    let duration_ms = start.elapsed().as_millis() as u64;

    match res {
        Ok(output) => {
            let exit_code = output.status.code().unwrap_or(-1);
            let stdout = truncate_output(&output.stdout, max_output_bytes);
            let stderr = truncate_output(&output.stderr, max_output_bytes);
            Ok(ExecutedCommand {
                exit_code,
                stdout,
                stderr,
                duration_ms,
            })
        }
        Err(e) => Err(VerifierError::ProcessError(format!(
            "Failed to execute '{} {:?}': {}",
            program, args, e
        ))),
    }
}

/// Pluggable Service Provider Interface (SPI) for language-specific build, test, and lint runners.
#[async_trait]
pub trait LanguageRunner: Send + Sync {
    /// Canonical runner name (e.g. "cargo", "pytest", "npm").
    fn name(&self) -> &'static str;

    /// Detects if this runner applies to the given workspace.
    fn detect(&self, workspace: &Path) -> bool;

    /// Runs compilation / type / syntax verification.
    async fn check(&self, workspace: &Path) -> Result<CompilationCheck, VerifierError>;

    /// Runs automated test suites and captures duration and test counts.
    async fn test(&self, workspace: &Path) -> Result<TestCheck, VerifierError>;

    /// Runs static linter analysis and counts warnings.
    async fn lint(&self, workspace: &Path) -> Result<LinterCheck, VerifierError>;
}

/// Cargo / Rust language runner.
pub struct CargoRunner;

#[async_trait]
impl LanguageRunner for CargoRunner {
    fn name(&self) -> &'static str {
        "cargo"
    }

    fn detect(&self, workspace: &Path) -> bool {
        workspace.join("Cargo.toml").is_file()
    }

    async fn check(&self, workspace: &Path) -> Result<CompilationCheck, VerifierError> {
        let exec = match run_command("cargo", &["check"], workspace, DEFAULT_MAX_OUTPUT_BYTES).await
        {
            Ok(e) => e,
            Err(e) => {
                return Ok(CompilationCheck {
                    status: StepStatus::Failed,
                    exit_code: -1,
                    message: Some(format!("Failed to spawn cargo check: {e}")),
                });
            }
        };

        let status = if exec.exit_code == 0 {
            StepStatus::Passed
        } else {
            StepStatus::Failed
        };

        let message = if exec.exit_code != 0 {
            let msg = if !exec.stderr.is_empty() {
                exec.stderr
            } else {
                exec.stdout
            };
            Some(msg)
        } else {
            None
        };

        Ok(CompilationCheck {
            status,
            exit_code: exec.exit_code,
            message,
        })
    }

    async fn test(&self, workspace: &Path) -> Result<TestCheck, VerifierError> {
        let exec = match run_command("cargo", &["test"], workspace, DEFAULT_MAX_OUTPUT_BYTES).await
        {
            Ok(e) => e,
            Err(e) => {
                return Ok(TestCheck {
                    status: StepStatus::Failed,
                    exit_code: Some(-1),
                    total: 0,
                    failed: 1,
                    duration_ms: Some(0),
                    message: Some(format!("Failed to spawn cargo test: {e}")),
                    flaky_quarantined: None,
                    retries: None,
                });
            }
        };

        let parsed = parse_cargo_test_output(&exec.stdout);
        let (total, failed) = match parsed {
            Some((t, f)) => (t, f),
            None => {
                if exec.exit_code == 0 {
                    (0, 0)
                } else {
                    (1, 1)
                }
            }
        };

        let status = if exec.exit_code == 0 && failed == 0 {
            StepStatus::Passed
        } else {
            StepStatus::Failed
        };

        let message = if status.is_failed() {
            let msg = if !exec.stderr.is_empty() {
                exec.stderr
            } else {
                exec.stdout
            };
            Some(msg)
        } else {
            None
        };

        Ok(TestCheck {
            status,
            exit_code: Some(exec.exit_code),
            total,
            failed,
            duration_ms: Some(exec.duration_ms),
            message,
            flaky_quarantined: None,
            retries: None,
        })
    }

    async fn lint(&self, workspace: &Path) -> Result<LinterCheck, VerifierError> {
        let args = ["clippy", "--all-targets", "--", "-D", "warnings"];
        let exec = match run_command("cargo", &args, workspace, DEFAULT_MAX_OUTPUT_BYTES).await {
            Ok(e) => e,
            Err(e) => {
                return Ok(LinterCheck {
                    status: StepStatus::Failed,
                    warnings: 1,
                    exit_code: Some(-1),
                    message: Some(format!("Failed to spawn cargo clippy: {e}")),
                });
            }
        };

        if exec.exit_code == 0 {
            Ok(LinterCheck {
                status: StepStatus::Passed,
                warnings: 0,
                exit_code: Some(0),
                message: None,
            })
        } else {
            let warnings_count = count_clippy_warnings(&exec.stderr).max(1);
            let message = if !exec.stderr.is_empty() {
                Some(exec.stderr)
            } else {
                Some(exec.stdout)
            };

            Ok(LinterCheck {
                status: StepStatus::Failed,
                warnings: warnings_count,
                exit_code: Some(exec.exit_code),
                message,
            })
        }
    }
}

/// Pytest / Python language runner.
pub struct PytestRunner;

#[async_trait]
impl LanguageRunner for PytestRunner {
    fn name(&self) -> &'static str {
        "pytest"
    }

    fn detect(&self, workspace: &Path) -> bool {
        workspace.join("pyproject.toml").is_file()
            || workspace.join("setup.py").is_file()
            || workspace.join("requirements.txt").is_file()
            || workspace.join("pytest.ini").is_file()
            || workspace.join("tox.ini").is_file()
            || workspace.join("setup.cfg").is_file()
    }

    async fn check(&self, workspace: &Path) -> Result<CompilationCheck, VerifierError> {
        // Python syntax check via compileall
        let res = run_command(
            "python3",
            &["-m", "compileall", "-q", "."],
            workspace,
            DEFAULT_MAX_OUTPUT_BYTES,
        )
        .await;
        let exec = match res {
            Ok(e) => e,
            Err(_) => {
                // Fallback to python
                match run_command(
                    "python",
                    &["-m", "compileall", "-q", "."],
                    workspace,
                    DEFAULT_MAX_OUTPUT_BYTES,
                )
                .await
                {
                    Ok(e) => e,
                    Err(e) => {
                        return Ok(CompilationCheck {
                            status: StepStatus::Failed,
                            exit_code: -1,
                            message: Some(format!("Failed to spawn python compileall: {e}")),
                        });
                    }
                }
            }
        };

        let status = if exec.exit_code == 0 {
            StepStatus::Passed
        } else {
            StepStatus::Failed
        };

        let message = if exec.exit_code != 0 {
            let msg = if !exec.stderr.is_empty() {
                exec.stderr
            } else {
                exec.stdout
            };
            Some(msg)
        } else {
            None
        };

        Ok(CompilationCheck {
            status,
            exit_code: exec.exit_code,
            message,
        })
    }

    async fn test(&self, workspace: &Path) -> Result<TestCheck, VerifierError> {
        let res = run_command("pytest", &[], workspace, DEFAULT_MAX_OUTPUT_BYTES).await;
        let exec = match res {
            Ok(e) => e,
            Err(_) => {
                // Fallback to python3 -m pytest
                match run_command(
                    "python3",
                    &["-m", "pytest"],
                    workspace,
                    DEFAULT_MAX_OUTPUT_BYTES,
                )
                .await
                {
                    Ok(e) => e,
                    Err(e) => {
                        return Ok(TestCheck {
                            status: StepStatus::Failed,
                            exit_code: Some(-1),
                            total: 0,
                            failed: 1,
                            duration_ms: Some(0),
                            message: Some(format!("Failed to spawn pytest: {e}")),
                            flaky_quarantined: None,
                            retries: None,
                        });
                    }
                }
            }
        };

        let parsed = parse_pytest_output(&exec.stdout);
        let (total, failed) = match parsed {
            Some((t, f)) => (t, f),
            None => {
                if exec.exit_code == 0 {
                    (0, 0)
                } else {
                    (1, 1)
                }
            }
        };

        let status = if exec.exit_code == 0 && failed == 0 {
            StepStatus::Passed
        } else {
            StepStatus::Failed
        };

        let message = if status.is_failed() {
            let msg = if !exec.stderr.is_empty() {
                exec.stderr
            } else {
                exec.stdout
            };
            Some(msg)
        } else {
            None
        };

        Ok(TestCheck {
            status,
            exit_code: Some(exec.exit_code),
            total,
            failed,
            duration_ms: Some(exec.duration_ms),
            message,
            flaky_quarantined: None,
            retries: None,
        })
    }

    async fn lint(&self, workspace: &Path) -> Result<LinterCheck, VerifierError> {
        // Try ruff check first, fallback to flake8
        let ruff_res =
            run_command("ruff", &["check", "."], workspace, DEFAULT_MAX_OUTPUT_BYTES).await;
        let exec = match ruff_res {
            Ok(e) => e,
            Err(_) => {
                match run_command("flake8", &["."], workspace, DEFAULT_MAX_OUTPUT_BYTES).await {
                    Ok(e) => e,
                    Err(e) => {
                        return Ok(LinterCheck {
                            status: StepStatus::Failed,
                            warnings: 1,
                            exit_code: Some(-1),
                            message: Some(format!(
                                "Neither ruff nor flake8 could be executed: {e}"
                            )),
                        });
                    }
                }
            }
        };

        if exec.exit_code == 0 {
            Ok(LinterCheck {
                status: StepStatus::Passed,
                warnings: 0,
                exit_code: Some(0),
                message: None,
            })
        } else {
            let message = if !exec.stderr.is_empty() {
                Some(exec.stderr)
            } else {
                Some(exec.stdout)
            };
            Ok(LinterCheck {
                status: StepStatus::Failed,
                warnings: 1,
                exit_code: Some(exec.exit_code),
                message,
            })
        }
    }
}

/// NPM / Node.js language runner.
pub struct NpmRunner;

#[async_trait]
impl LanguageRunner for NpmRunner {
    fn name(&self) -> &'static str {
        "npm"
    }

    fn detect(&self, workspace: &Path) -> bool {
        workspace.join("package.json").is_file()
    }

    async fn check(&self, workspace: &Path) -> Result<CompilationCheck, VerifierError> {
        // If tsconfig.json is present, run tsc --noEmit
        if workspace.join("tsconfig.json").is_file() {
            let exec = match run_command(
                "npx",
                &["--no-install", "tsc", "--noEmit"],
                workspace,
                DEFAULT_MAX_OUTPUT_BYTES,
            )
            .await
            {
                Ok(e) => e,
                Err(e) => {
                    return Ok(CompilationCheck {
                        status: StepStatus::Failed,
                        exit_code: -1,
                        message: Some(format!("Failed to execute tsc check: {e}")),
                    });
                }
            };
            let status = if exec.exit_code == 0 {
                StepStatus::Passed
            } else {
                StepStatus::Failed
            };
            let message = if exec.exit_code != 0 {
                Some(if !exec.stderr.is_empty() {
                    exec.stderr
                } else {
                    exec.stdout
                })
            } else {
                None
            };
            return Ok(CompilationCheck {
                status,
                exit_code: exec.exit_code,
                message,
            });
        }

        // Try npm run check or typecheck
        let has_check_script = check_package_json_has_script(workspace, "check")
            || check_package_json_has_script(workspace, "typecheck");

        if has_check_script {
            let script = if check_package_json_has_script(workspace, "check") {
                "check"
            } else {
                "typecheck"
            };
            let exec =
                match run_command("npm", &["run", script], workspace, DEFAULT_MAX_OUTPUT_BYTES)
                    .await
                {
                    Ok(e) => e,
                    Err(e) => {
                        return Ok(CompilationCheck {
                            status: StepStatus::Failed,
                            exit_code: -1,
                            message: Some(format!("Failed to execute npm run {script}: {e}")),
                        });
                    }
                };
            let status = if exec.exit_code == 0 {
                StepStatus::Passed
            } else {
                StepStatus::Failed
            };
            let message = if exec.exit_code != 0 {
                Some(if !exec.stderr.is_empty() {
                    exec.stderr
                } else {
                    exec.stdout
                })
            } else {
                None
            };
            Ok(CompilationCheck {
                status,
                exit_code: exec.exit_code,
                message,
            })
        } else {
            // No type check configured; passes trivially
            Ok(CompilationCheck {
                status: StepStatus::Passed,
                exit_code: 0,
                message: None,
            })
        }
    }

    async fn test(&self, workspace: &Path) -> Result<TestCheck, VerifierError> {
        let exec = match run_command("npm", &["test"], workspace, DEFAULT_MAX_OUTPUT_BYTES).await {
            Ok(e) => e,
            Err(e) => {
                return Ok(TestCheck {
                    status: StepStatus::Failed,
                    exit_code: Some(-1),
                    total: 0,
                    failed: 1,
                    duration_ms: Some(0),
                    message: Some(format!("Failed to execute npm test: {e}")),
                    flaky_quarantined: None,
                    retries: None,
                });
            }
        };

        let status = if exec.exit_code == 0 {
            StepStatus::Passed
        } else {
            StepStatus::Failed
        };

        let (total, failed) = if exec.exit_code == 0 { (0, 0) } else { (1, 1) };

        let message = if status.is_failed() {
            Some(if !exec.stderr.is_empty() {
                exec.stderr
            } else {
                exec.stdout
            })
        } else {
            None
        };

        Ok(TestCheck {
            status,
            exit_code: Some(exec.exit_code),
            total,
            failed,
            duration_ms: Some(exec.duration_ms),
            message,
            flaky_quarantined: None,
            retries: None,
        })
    }

    async fn lint(&self, workspace: &Path) -> Result<LinterCheck, VerifierError> {
        let has_lint = check_package_json_has_script(workspace, "lint");
        let (cmd, args): (&str, &[&str]) = if has_lint {
            ("npm", &["run", "lint"])
        } else {
            ("npx", &["--no-install", "eslint", "."])
        };

        let exec = match run_command(cmd, args, workspace, DEFAULT_MAX_OUTPUT_BYTES).await {
            Ok(e) => e,
            Err(e) => {
                return Ok(LinterCheck {
                    status: StepStatus::Failed,
                    warnings: 1,
                    exit_code: Some(-1),
                    message: Some(format!("Failed to execute npm lint: {e}")),
                });
            }
        };

        if exec.exit_code == 0 {
            Ok(LinterCheck {
                status: StepStatus::Passed,
                warnings: 0,
                exit_code: Some(0),
                message: None,
            })
        } else {
            let message = if !exec.stderr.is_empty() {
                Some(exec.stderr)
            } else {
                Some(exec.stdout)
            };
            Ok(LinterCheck {
                status: StepStatus::Failed,
                warnings: 1,
                exit_code: Some(exec.exit_code),
                message,
            })
        }
    }
}

fn check_package_json_has_script(workspace: &Path, script_name: &str) -> bool {
    let pkg_file = workspace.join("package.json");
    if let Ok(content) = std::fs::read_to_string(pkg_file) {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
            return val
                .get("scripts")
                .and_then(|s| s.get(script_name))
                .is_some();
        }
    }
    false
}

/// Registry of pluggable language runners with priority detection and name lookup.
pub struct RunnerRegistry {
    runners: Vec<Box<dyn LanguageRunner>>,
}

impl RunnerRegistry {
    /// Creates a new `RunnerRegistry` populated with default runners in priority order:
    /// 1. `CargoRunner` (Rust)
    /// 2. `PytestRunner` (Python)
    /// 3. `NpmRunner` (Node.js)
    pub fn new() -> Self {
        Self {
            runners: vec![
                Box::new(CargoRunner),
                Box::new(PytestRunner),
                Box::new(NpmRunner),
            ],
        }
    }

    /// Creates an empty registry without pre-registered runners.
    pub fn empty() -> Self {
        Self {
            runners: Vec::new(),
        }
    }

    /// Registers a runner at the end of the priority list.
    pub fn register(&mut self, runner: Box<dyn LanguageRunner>) {
        self.runners.push(runner);
    }

    /// Registers a runner at the front of the priority list (highest priority).
    pub fn register_front(&mut self, runner: Box<dyn LanguageRunner>) {
        self.runners.insert(0, runner);
    }

    /// Looks up a runner by name (case-insensitive).
    pub fn get_by_name(&self, name: &str) -> Option<&dyn LanguageRunner> {
        let trimmed = name.trim();
        self.runners
            .iter()
            .find(|r| r.name().eq_ignore_ascii_case(trimmed))
            .map(|b| b.as_ref())
    }

    /// Auto-detects the first suitable runner for the given workspace in priority order.
    pub fn detect_runner(&self, workspace: &Path) -> Option<&dyn LanguageRunner> {
        self.runners
            .iter()
            .find(|r| r.detect(workspace))
            .map(|b| b.as_ref())
    }

    /// Returns slice of all registered runners.
    pub fn runners(&self) -> &[Box<dyn LanguageRunner>] {
        &self.runners
    }

    /// Returns the list of registered runner names.
    pub fn runner_names(&self) -> Vec<&'static str> {
        self.runners.iter().map(|r| r.name()).collect()
    }
}

impl Default for RunnerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Parses standard `cargo test` stdout lines to extract total and failed counts.
pub fn parse_cargo_test_output(stdout: &str) -> Option<(usize, usize)> {
    let re =
        Regex::new(r"test result:\s+(?:ok|FAILED)\.\s+(\d+)\s+passed;\s+(\d+)\s+failed;").ok()?;
    let mut total_passed = 0;
    let mut total_failed = 0;
    let mut matched = false;

    for caps in re.captures_iter(stdout) {
        if let (Some(p_match), Some(f_match)) = (caps.get(1), caps.get(2)) {
            let p: usize = p_match.as_str().parse().unwrap_or(0);
            let f: usize = f_match.as_str().parse().unwrap_or(0);
            total_passed += p;
            total_failed += f;
            matched = true;
        }
    }

    if matched {
        Some((total_passed + total_failed, total_failed))
    } else {
        None
    }
}

/// Counts warnings/errors in clippy output.
pub fn count_clippy_warnings(output: &str) -> usize {
    let mut count = 0;
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("warning:")
            || (trimmed.starts_with("error:")
                && !trimmed.contains("could not compile")
                && !trimmed.contains("aborting due to"))
        {
            count += 1;
        }
    }
    count
}

/// Parses pytest output summary to extract total and failed counts.
pub fn parse_pytest_output(output: &str) -> Option<(usize, usize)> {
    let re_passed = Regex::new(r"(\d+)\s+passed").ok()?;
    let re_failed = Regex::new(r"(\d+)\s+failed").ok()?;
    let re_errors = Regex::new(r"(\d+)\s+errors?").ok()?;

    let mut passed = 0;
    let mut failed = 0;
    let mut matched = false;

    for caps in re_passed.captures_iter(output) {
        if let Some(m) = caps.get(1) {
            passed += m.as_str().parse::<usize>().unwrap_or(0);
            matched = true;
        }
    }
    for caps in re_failed.captures_iter(output) {
        if let Some(m) = caps.get(1) {
            failed += m.as_str().parse::<usize>().unwrap_or(0);
            matched = true;
        }
    }
    for caps in re_errors.captures_iter(output) {
        if let Some(m) = caps.get(1) {
            failed += m.as_str().parse::<usize>().unwrap_or(0);
            matched = true;
        }
    }

    if matched {
        Some((passed + failed, failed))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_truncate_output_short() {
        let bytes = b"short text";
        assert_eq!(truncate_output(bytes, 100), "short text");
    }

    #[test]
    fn test_truncate_output_long() {
        let bytes = b"1234567890abcdefghij";
        let truncated = truncate_output(bytes, 10);
        assert!(truncated.starts_with("1234567890"));
        assert!(truncated.contains("... [truncated, total 20 bytes]"));
    }

    #[test]
    fn test_truncate_output_utf8_multibyte() {
        // "你好世界" is 12 bytes (3 bytes per character)
        let s = "你好世界";
        let bytes = s.as_bytes();
        // Truncate at 7 bytes (which falls inside the 3rd character "世" at bytes 6..9)
        let truncated = truncate_output(bytes, 7);
        // It must cleanly slice at 6 bytes ("你好") and not corrupt UTF-8
        assert!(truncated.starts_with("你好"));
        assert!(truncated.contains("... [truncated, total 12 bytes]"));
    }

    #[test]
    fn test_runner_detection() {
        let temp = TempDir::new().unwrap();
        let cargo = CargoRunner;
        let pytest = PytestRunner;
        let npm = NpmRunner;

        // Empty workspace
        assert!(!cargo.detect(temp.path()));
        assert!(!pytest.detect(temp.path()));
        assert!(!npm.detect(temp.path()));

        // Rust workspace
        std::fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\n",
        )
        .unwrap();
        assert!(cargo.detect(temp.path()));
        assert_eq!(cargo.name(), "cargo");

        // Python workspace
        let py_temp = TempDir::new().unwrap();
        std::fs::write(py_temp.path().join("pyproject.toml"), "").unwrap();
        assert!(pytest.detect(py_temp.path()));
        assert_eq!(pytest.name(), "pytest");

        // NPM workspace
        let npm_temp = TempDir::new().unwrap();
        std::fs::write(npm_temp.path().join("package.json"), "{}").unwrap();
        assert!(npm.detect(npm_temp.path()));
        assert_eq!(npm.name(), "npm");
    }

    #[test]
    fn test_runner_registry() {
        let registry = RunnerRegistry::new();
        assert_eq!(registry.runner_names(), vec!["cargo", "pytest", "npm"]);

        // Name lookup (case-insensitive)
        assert!(registry.get_by_name("cargo").is_some());
        assert!(registry.get_by_name("CARGO").is_some());
        assert!(registry.get_by_name("pytest").is_some());
        assert!(registry.get_by_name("Npm").is_some());
        assert!(registry.get_by_name("unknown").is_none());

        // Detection in registry
        let temp = TempDir::new().unwrap();
        std::fs::write(temp.path().join("requirements.txt"), "pytest\n").unwrap();
        let detected = registry
            .detect_runner(temp.path())
            .expect("runner detected");
        assert_eq!(detected.name(), "pytest");
    }

    #[test]
    fn test_parse_cargo_test_output() {
        let sample = r#"
running 3 tests
test test_a ... ok
test test_b ... FAILED
test test_c ... ok

failures:

test result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s
"#;
        let (total, failed) = parse_cargo_test_output(sample).unwrap();
        assert_eq!(total, 3);
        assert_eq!(failed, 1);
    }

    #[test]
    fn test_parse_pytest_output() {
        let sample = "=== 12 passed, 2 failed in 0.35s ===";
        let (total, failed) = parse_pytest_output(sample).unwrap();
        assert_eq!(total, 14);
        assert_eq!(failed, 2);

        let pass_only = "=== 5 passed in 0.02s ===";
        let (total, failed) = parse_pytest_output(pass_only).unwrap();
        assert_eq!(total, 5);
        assert_eq!(failed, 0);
    }

    #[test]
    fn test_count_clippy_warnings() {
        let clippy_output = r#"
warning: unused variable: `x`
  --> src/main.rs:10:5

error: variable does not need to be mutable
  --> src/main.rs:15:9

error: could not compile `demo` due to 2 previous errors
"#;
        let count = count_clippy_warnings(clippy_output);
        assert_eq!(count, 2);
    }

    #[test]
    fn test_runner_registry_empty_and_custom_registration() {
        let mut registry = RunnerRegistry::empty();
        assert!(registry.runners().is_empty());
        assert!(registry.runner_names().is_empty());

        registry.register(Box::new(PytestRunner));
        assert_eq!(registry.runner_names(), vec!["pytest"]);

        registry.register_front(Box::new(CargoRunner));
        assert_eq!(registry.runner_names(), vec!["cargo", "pytest"]);
    }

    #[test]
    fn test_parse_cargo_test_output_multi_suite() {
        let multi = r#"
running 2 tests
test test_one ... ok
test test_two ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s

running 3 tests
test test_three ... ok
test test_four ... FAILED
test test_five ... ok

test result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
"#;
        let (total, failed) = parse_cargo_test_output(multi).unwrap();
        assert_eq!(total, 5);
        assert_eq!(failed, 1);
    }

    #[test]
    fn test_parse_cargo_test_output_zero_tests() {
        let sample = "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s";
        let (total, failed) = parse_cargo_test_output(sample).unwrap();
        assert_eq!(total, 0);
        assert_eq!(failed, 0);
    }

    #[test]
    fn test_parse_pytest_output_with_errors() {
        let sample = "=== 8 passed, 1 failed, 2 errors in 1.20s ===";
        let (total, failed) = parse_pytest_output(sample).unwrap();
        assert_eq!(total, 11);
        assert_eq!(failed, 3);
    }

    #[test]
    fn test_cargo_runner_real_workspace() {
        let runner = CargoRunner;
        let cwd = std::env::current_dir().unwrap();
        assert!(runner.detect(&cwd));
        assert_eq!(runner.name(), "cargo");
    }

    #[test]
    fn test_check_package_json_has_script() {
        let temp = TempDir::new().unwrap();
        let pkg = r#"
{
  "name": "test-pkg",
  "scripts": {
    "test": "jest",
    "lint": "eslint ."
  }
}
"#;
        std::fs::write(temp.path().join("package.json"), pkg).unwrap();
        assert!(check_package_json_has_script(temp.path(), "test"));
        assert!(check_package_json_has_script(temp.path(), "lint"));
        assert!(!check_package_json_has_script(temp.path(), "build"));
    }
}
