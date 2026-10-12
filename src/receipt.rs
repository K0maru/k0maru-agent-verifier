use crate::error::VerifierError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::fmt;
use std::path::PathBuf;

pub const DEFAULT_RECEIPT_SCHEMA: &str = "https://k0maru.dev/schemas/agent-receipt-v1.json";

/// Overall verdict of the downstream task execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerdictStatus {
    Success,
    Failed,
    Blocked,
}

impl VerdictStatus {
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success)
    }

    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed)
    }

    pub fn is_blocked(&self) -> bool {
        matches!(self, Self::Blocked)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failed => "failed",
            Self::Blocked => "blocked",
        }
    }
}

impl fmt::Display for VerdictStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl Serialize for VerdictStatus {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for VerdictStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.trim().to_ascii_lowercase().as_str() {
            "success" => Ok(Self::Success),
            "failed" => Ok(Self::Failed),
            "blocked" => Ok(Self::Blocked),
            other => Err(serde::de::Error::custom(format!(
                "Invalid verdict status: '{other}'. Expected 'success', 'failed', or 'blocked'"
            ))),
        }
    }
}

/// Status of an individual check step (compilation, test, lint, scope_guard).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepStatus {
    Passed,
    Failed,
    Skipped,
}

impl StepStatus {
    pub fn is_passed(&self) -> bool {
        matches!(self, Self::Passed)
    }

    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed)
    }

    pub fn is_skipped(&self) -> bool {
        matches!(self, Self::Skipped)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }
}

impl fmt::Display for StepStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl Serialize for StepStatus {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for StepStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.trim().to_ascii_lowercase().as_str() {
            "passed" => Ok(Self::Passed),
            "failed" => Ok(Self::Failed),
            "skipped" => Ok(Self::Skipped),
            other => Err(serde::de::Error::custom(format!(
                "Invalid step status: '{other}'. Expected 'passed', 'failed', or 'skipped'"
            ))),
        }
    }
}

/// Compilation/syntax check result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompilationCheck {
    pub status: StepStatus,
    pub exit_code: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl CompilationCheck {
    pub fn is_passed(&self) -> bool {
        self.status.is_passed() && self.exit_code == 0
    }
}

/// Test runner execution check result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestCheck {
    pub status: StepStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub total: usize,
    pub failed: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl TestCheck {
    pub fn is_passed(&self) -> bool {
        self.status.is_passed() && self.failed == 0 && self.exit_code.unwrap_or(0) == 0
    }
}

/// Linter and static analysis check result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinterCheck {
    pub status: StepStatus,
    pub warnings: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl LinterCheck {
    pub fn is_passed(&self) -> bool {
        self.status.is_passed() && self.warnings == 0 && self.exit_code.unwrap_or(0) == 0
    }
}

/// Scope guard check result ensuring changes are restricted to declared whitelist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeGuardCheck {
    pub status: StepStatus,
    #[serde(default)]
    pub modified_files: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unauthorized_files: Vec<String>,
}

impl ScopeGuardCheck {
    pub fn is_passed(&self) -> bool {
        self.status.is_passed() && self.unauthorized_files.is_empty()
    }
}

/// Backward compatibility aliases.
pub type StepResult = CompilationCheck;
pub type TestStepResult = TestCheck;
pub type ScopeGuardResult = ScopeGuardCheck;

/// Aggregated checks summary for compilation, tests, linter, and scope guard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckSummary {
    pub compilation: CompilationCheck,
    pub tests: TestCheck,
    pub linter: LinterCheck,
    pub scope_guard: ScopeGuardCheck,
}

impl CheckSummary {
    pub fn is_all_passed(&self) -> bool {
        self.compilation.is_passed()
            && self.tests.is_passed()
            && self.linter.is_passed()
            && self.scope_guard.is_passed()
    }
}

/// Execution artifacts including Git diff SHA-256 fingerprint and timestamp.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ArtifactSummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_patch_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt_timestamp: Option<String>,
}

impl ArtifactSummary {
    pub fn new(
        git_patch_sha256: impl Into<Option<String>>,
        receipt_timestamp: impl Into<Option<String>>,
    ) -> Self {
        Self {
            git_patch_sha256: git_patch_sha256.into(),
            receipt_timestamp: receipt_timestamp.into(),
        }
    }
}

fn default_receipt_schema() -> String {
    DEFAULT_RECEIPT_SCHEMA.to_string()
}

/// Execution receipt conforming to `https://k0maru.dev/schemas/agent-receipt-v1.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionReceipt {
    #[serde(rename = "$schema", default = "default_receipt_schema")]
    pub schema: String,
    pub task_id: String,
    pub workspace: PathBuf,
    pub verdict: VerdictStatus,
    pub checks: CheckSummary,
    #[serde(default)]
    pub artifacts: ArtifactSummary,
}

impl fmt::Display for ExecutionReceipt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.summary())
    }
}

impl ExecutionReceipt {
    /// Computes the SHA-256 hex digest of the given byte slice.
    pub fn compute_sha256(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        hex::encode(hasher.finalize())
    }

    /// Verifies the given patch bytes against `artifacts.git_patch_sha256`.
    ///
    /// If `artifacts.git_patch_sha256` is `None`, returns `Ok(true)`.
    pub fn verify_patch_sha256(&self, patch_bytes: &[u8]) -> Result<bool, VerifierError> {
        let expected = match &self.artifacts.git_patch_sha256 {
            Some(sha) => sha.trim(),
            None => return Ok(true),
        };

        if expected.len() != 64 || hex::decode(expected).is_err() {
            return Err(VerifierError::InvalidReceipt(format!(
                "Invalid git_patch_sha256 format: expected 64 hex characters, got '{expected}'"
            )));
        }

        let actual = Self::compute_sha256(patch_bytes);
        Ok(actual.eq_ignore_ascii_case(expected))
    }

    /// Returns `true` if the verdict is `Success` and all checks passed.
    pub fn is_passing(&self) -> bool {
        self.verdict.is_success() && self.checks.is_all_passed()
    }

    /// Validates the internal consistency and schema invariants of the receipt.
    pub fn validate(&self) -> Result<(), VerifierError> {
        let task_id = self.task_id.trim();
        if task_id.is_empty() {
            return Err(VerifierError::InvalidReceipt(
                "Receipt 'task_id' cannot be empty".to_string(),
            ));
        }

        // Reject path traversal tokens, slashes, or null bytes
        if task_id.contains("..")
            || task_id.contains('/')
            || task_id.contains('\\')
            || task_id.contains('\0')
        {
            return Err(VerifierError::InvalidReceipt(format!(
                "Receipt 'task_id' contains invalid path characters: '{task_id}'"
            )));
        }

        if self.workspace.as_os_str().is_empty() {
            return Err(VerifierError::InvalidReceipt(
                "Receipt 'workspace' path cannot be empty".to_string(),
            ));
        }

        // Validate compilation check consistency
        if self.checks.compilation.status.is_passed() && self.checks.compilation.exit_code != 0 {
            return Err(VerifierError::InvalidReceipt(format!(
                "Compilation marked 'passed' but exit_code is non-zero ({})",
                self.checks.compilation.exit_code
            )));
        }

        // Validate tests check consistency
        if self.checks.tests.status.is_passed() && self.checks.tests.failed > 0 {
            return Err(VerifierError::InvalidReceipt(format!(
                "Tests marked 'passed' but failed count is non-zero ({})",
                self.checks.tests.failed
            )));
        }
        if self.checks.tests.failed > self.checks.tests.total {
            return Err(VerifierError::InvalidReceipt(format!(
                "Tests failed count ({}) exceeds total test count ({})",
                self.checks.tests.failed, self.checks.tests.total
            )));
        }

        // Validate linter check consistency
        if self.checks.linter.status.is_passed() && self.checks.linter.warnings > 0 {
            return Err(VerifierError::InvalidReceipt(format!(
                "Linter marked 'passed' but warnings count is non-zero ({})",
                self.checks.linter.warnings
            )));
        }

        // Validate scope guard check consistency
        if self.checks.scope_guard.status.is_passed()
            && !self.checks.scope_guard.unauthorized_files.is_empty()
        {
            return Err(VerifierError::InvalidReceipt(format!(
                "Scope guard marked 'passed' but unauthorized_files is non-empty: {:?}",
                self.checks.scope_guard.unauthorized_files
            )));
        }

        // Validate verdict consistency
        if self.verdict.is_success() && !self.checks.is_all_passed() {
            return Err(VerifierError::InvalidReceipt(
                "Verdict is marked 'success' but not all checks passed".to_string(),
            ));
        }

        // Validate timestamp format if provided
        if let Some(ref ts) = self.artifacts.receipt_timestamp {
            chrono::DateTime::parse_from_rfc3339(ts).map_err(|e| {
                VerifierError::InvalidReceipt(format!(
                    "Invalid receipt_timestamp RFC-3339 format '{ts}': {e}"
                ))
            })?;
        }

        // Validate git_patch_sha256 format if provided
        if let Some(ref sha) = self.artifacts.git_patch_sha256 {
            let trimmed = sha.trim();
            if trimmed.len() != 64 || hex::decode(trimmed).is_err() {
                return Err(VerifierError::InvalidReceipt(format!(
                    "Invalid git_patch_sha256 format: expected 64 hexadecimal characters, got '{sha}'"
                )));
            }
        }

        Ok(())
    }

    /// Formats a human-readable summary of the execution receipt.
    pub fn summary(&self) -> String {
        let pass_status = if self.is_passing() {
            "PASSING"
        } else {
            "FAILING"
        };
        let comp_msg = match &self.checks.compilation.message {
            Some(m) if !m.is_empty() => format!(" - message: {}", m),
            _ => String::new(),
        };
        format!(
            "Receipt [{} | {}]\n  Workspace: {}\n  Verdict: {}\n  Checks:\n    - compilation: {} (exit {}){}\n    - tests: {} ({}/{} failed, duration: {}ms)\n    - linter: {} ({} warnings)\n    - scope_guard: {} ({} modified, {} unauthorized)\n  Artifacts:\n    - patch sha256: {}\n    - timestamp: {}",
            self.task_id,
            pass_status,
            self.workspace.display(),
            self.verdict,
            self.checks.compilation.status,
            self.checks.compilation.exit_code,
            comp_msg,
            self.checks.tests.status,
            self.checks.tests.failed,
            self.checks.tests.total,
            self.checks.tests.duration_ms.unwrap_or(0),
            self.checks.linter.status,
            self.checks.linter.warnings,
            self.checks.scope_guard.status,
            self.checks.scope_guard.modified_files.len(),
            self.checks.scope_guard.unauthorized_files.len(),
            self.artifacts.git_patch_sha256.as_deref().unwrap_or("<none>"),
            self.artifacts.receipt_timestamp.as_deref().unwrap_or("<none>"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verdict_status_case_insensitive_serde() {
        let success_json = "\"sUcCeSs\"";
        let verdict: VerdictStatus = serde_json::from_str(success_json).expect("deserialize");
        assert_eq!(verdict, VerdictStatus::Success);
        assert!(verdict.is_success());
        assert!(!verdict.is_failed());
        assert_eq!(serde_json::to_string(&verdict).unwrap(), "\"success\"");

        let failed_json = "\"  FAILED  \"";
        let verdict: VerdictStatus = serde_json::from_str(failed_json).expect("deserialize");
        assert_eq!(verdict, VerdictStatus::Failed);
        assert!(verdict.is_failed());

        let blocked_json = "\"Blocked\"";
        let verdict: VerdictStatus = serde_json::from_str(blocked_json).expect("deserialize");
        assert_eq!(verdict, VerdictStatus::Blocked);
        assert!(verdict.is_blocked());

        let invalid_json = "\"unknown_status\"";
        assert!(serde_json::from_str::<VerdictStatus>(invalid_json).is_err());
    }

    #[test]
    fn test_step_status_case_insensitive_serde() {
        let passed_json = "\"PASSED\"";
        let status: StepStatus = serde_json::from_str(passed_json).expect("deserialize");
        assert_eq!(status, StepStatus::Passed);
        assert!(status.is_passed());
        assert_eq!(serde_json::to_string(&status).unwrap(), "\"passed\"");

        let failed_json = "\"Failed\"";
        let status: StepStatus = serde_json::from_str(failed_json).expect("deserialize");
        assert_eq!(status, StepStatus::Failed);
        assert!(status.is_failed());

        let skipped_json = "\"skipped\"";
        let status: StepStatus = serde_json::from_str(skipped_json).expect("deserialize");
        assert_eq!(status, StepStatus::Skipped);
        assert!(status.is_skipped());

        let invalid_json = "\"running\"";
        assert!(serde_json::from_str::<StepStatus>(invalid_json).is_err());
    }

    #[test]
    fn test_compute_sha256() {
        assert_eq!(
            ExecutionReceipt::compute_sha256(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        let digest = ExecutionReceipt::compute_sha256(b"hello world\n");
        assert_eq!(digest.len(), 64);
    }

    fn sample_valid_receipt() -> ExecutionReceipt {
        ExecutionReceipt {
            schema: DEFAULT_RECEIPT_SCHEMA.to_string(),
            task_id: "task-01".to_string(),
            workspace: PathBuf::from("/tmp/repo"),
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
                    total: 10,
                    failed: 0,
                    duration_ms: Some(150),
                    message: None,
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
                },
            },
            artifacts: ArtifactSummary {
                git_patch_sha256: Some(
                    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string(),
                ),
                receipt_timestamp: Some("2026-10-12T15:00:00Z".to_string()),
            },
        }
    }

    #[test]
    fn test_receipt_serde_and_schema_conformance() {
        let receipt = sample_valid_receipt();
        let json = serde_json::to_string_pretty(&receipt).expect("serialize");

        assert!(json.contains("\"$schema\": \"https://k0maru.dev/schemas/agent-receipt-v1.json\""));
        assert!(json.contains("\"verdict\": \"success\""));
        assert!(json.contains("\"compilation\":"));
        assert!(json.contains("\"tests\":"));
        assert!(json.contains("\"linter\":"));
        assert!(json.contains("\"scope_guard\":"));

        let deserialized: ExecutionReceipt = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(receipt, deserialized);
        assert!(deserialized.is_passing());
        assert!(deserialized.validate().is_ok());
    }

    #[test]
    fn test_receipt_validate_invalid_cases() {
        // Empty task_id
        let mut r = sample_valid_receipt();
        r.task_id = "   ".to_string();
        assert!(r.validate().is_err());

        // Path traversal in task_id
        let mut r = sample_valid_receipt();
        r.task_id = "task/../sub".to_string();
        assert!(r.validate().is_err());

        // Empty workspace
        let mut r = sample_valid_receipt();
        r.workspace = PathBuf::from("");
        assert!(r.validate().is_err());

        // Inconsistent compilation
        let mut r = sample_valid_receipt();
        r.checks.compilation.status = StepStatus::Passed;
        r.checks.compilation.exit_code = 1;
        assert!(r.validate().is_err());

        // Inconsistent tests
        let mut r = sample_valid_receipt();
        r.checks.tests.status = StepStatus::Passed;
        r.checks.tests.failed = 2;
        assert!(r.validate().is_err());

        // Tests failed > total
        let mut r = sample_valid_receipt();
        r.checks.tests.total = 2;
        r.checks.tests.failed = 5;
        assert!(r.validate().is_err());

        // Inconsistent linter
        let mut r = sample_valid_receipt();
        r.checks.linter.status = StepStatus::Passed;
        r.checks.linter.warnings = 3;
        assert!(r.validate().is_err());

        // Inconsistent scope guard
        let mut r = sample_valid_receipt();
        r.checks.scope_guard.status = StepStatus::Passed;
        r.checks.scope_guard.unauthorized_files = vec!["secret.env".to_string()];
        assert!(r.validate().is_err());

        // Success verdict when checks failed
        let mut r = sample_valid_receipt();
        r.checks.tests.status = StepStatus::Failed;
        r.checks.tests.failed = 1;
        r.verdict = VerdictStatus::Success;
        assert!(r.validate().is_err());

        // Invalid timestamp
        let mut r = sample_valid_receipt();
        r.artifacts.receipt_timestamp = Some("invalid-date".to_string());
        assert!(r.validate().is_err());

        // Invalid patch SHA
        let mut r = sample_valid_receipt();
        r.artifacts.git_patch_sha256 = Some("not-hex-and-too-short".to_string());
        assert!(r.validate().is_err());
    }

    #[test]
    fn test_verify_patch_sha256() {
        let mut r = sample_valid_receipt();
        let bytes = b"patch content\n";
        let digest = ExecutionReceipt::compute_sha256(bytes);
        r.artifacts.git_patch_sha256 = Some(digest);

        assert!(r.verify_patch_sha256(bytes).unwrap());
        assert!(!r.verify_patch_sha256(b"different content").unwrap());

        // None sha256 always returns true
        r.artifacts.git_patch_sha256 = None;
        assert!(r.verify_patch_sha256(b"anything").unwrap());
    }

    #[test]
    fn test_summary_and_display() {
        let receipt = sample_valid_receipt();
        let summary = receipt.summary();
        assert!(summary.contains("Receipt [task-01 | PASSING]"));
        assert!(summary.contains("Workspace: /tmp/repo"));
        assert!(summary.contains("Verdict: success"));
        assert_eq!(format!("{}", receipt), summary);
    }

    #[test]
    fn test_receipt_blocked_verdict_validation() {
        let mut r = sample_valid_receipt();
        r.verdict = VerdictStatus::Blocked;
        r.checks.compilation.status = StepStatus::Failed;
        r.checks.compilation.exit_code = 1;
        assert!(r.validate().is_ok());
        assert!(!r.is_passing());
    }

    #[test]
    fn test_receipt_optional_fields_and_empty_artifacts() {
        let mut r = sample_valid_receipt();
        r.checks.compilation.message = None;
        r.checks.tests.duration_ms = None;
        r.checks.tests.exit_code = None;
        r.checks.tests.message = None;
        r.checks.linter.exit_code = None;
        r.checks.linter.message = None;
        r.artifacts = ArtifactSummary::default();

        assert!(r.validate().is_ok());
        let json = serde_json::to_string(&r).unwrap();
        let deserialized: ExecutionReceipt = serde_json::from_str(&json).unwrap();
        assert_eq!(r, deserialized);
        assert!(deserialized.artifacts.git_patch_sha256.is_none());
        assert!(deserialized.artifacts.receipt_timestamp.is_none());
    }

    #[test]
    fn test_check_individual_is_passed_logic() {
        let comp_pass = CompilationCheck {
            status: StepStatus::Passed,
            exit_code: 0,
            message: None,
        };
        assert!(comp_pass.is_passed());

        let comp_fail = CompilationCheck {
            status: StepStatus::Passed,
            exit_code: 1,
            message: None,
        };
        assert!(!comp_fail.is_passed());

        let test_pass = TestCheck {
            status: StepStatus::Passed,
            exit_code: Some(0),
            total: 5,
            failed: 0,
            duration_ms: None,
            message: None,
        };
        assert!(test_pass.is_passed());

        let test_fail = TestCheck {
            status: StepStatus::Passed,
            exit_code: Some(0),
            total: 5,
            failed: 1,
            duration_ms: None,
            message: None,
        };
        assert!(!test_fail.is_passed());

        let lint_pass = LinterCheck {
            status: StepStatus::Passed,
            warnings: 0,
            exit_code: None,
            message: None,
        };
        assert!(lint_pass.is_passed());

        let lint_fail = LinterCheck {
            status: StepStatus::Passed,
            warnings: 2,
            exit_code: None,
            message: None,
        };
        assert!(!lint_fail.is_passed());

        let scope_pass = ScopeGuardCheck {
            status: StepStatus::Passed,
            modified_files: vec!["foo.rs".into()],
            unauthorized_files: vec![],
        };
        assert!(scope_pass.is_passed());

        let scope_fail = ScopeGuardCheck {
            status: StepStatus::Passed,
            modified_files: vec!["foo.rs".into()],
            unauthorized_files: vec!["bar.rs".into()],
        };
        assert!(!scope_fail.is_passed());
    }

    #[test]
    fn test_backward_compatibility_aliases() {
        let s: StepResult = CompilationCheck {
            status: StepStatus::Passed,
            exit_code: 0,
            message: None,
        };
        assert!(s.is_passed());

        let t: TestStepResult = TestCheck {
            status: StepStatus::Passed,
            exit_code: Some(0),
            total: 1,
            failed: 0,
            duration_ms: None,
            message: None,
        };
        assert!(t.is_passed());

        let g: ScopeGuardResult = ScopeGuardCheck {
            status: StepStatus::Passed,
            modified_files: vec![],
            unauthorized_files: vec![],
        };
        assert!(g.is_passed());
    }
}
