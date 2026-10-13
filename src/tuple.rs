//! Linda Tuple Space native emitter and models for K0maru Fabric.
//!
//! Provides decoupled atomic coordination via Linda primitives (`out`, `in`, `rd`)
//! physically persisted to `~/.k0maru/tuples/`.

use crate::constants::{
    DEFAULT_K0MARU_DIR, DEFAULT_TUPLES_SUBDIR, DEFAULT_TUPLE_DOMAIN, ENV_K0MARU_TUPLE_DIR,
    TUPLE_KIND_BLOCKED, TUPLE_KIND_VERIFIED,
};
use crate::error::VerifierError;
use crate::receipt::{ExecutionReceipt, StepStatus, VerdictStatus};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Linda Tuple envelope matching k0maru-router tuple space specification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LindaTuple {
    /// Unique tuple identifier (e.g. `tup_b9ebc4b1d5bd426698fc3a62dbd5d00c`).
    pub id: String,
    /// Tuple classification or event kind (e.g. `TASK_VERIFIED`, `TASK_BLOCKED`).
    pub kind: String,
    /// Target repository or domain identifier (e.g. `VERIFIER`, `AUTH-SRV`).
    pub domain: String,
    /// Associated task identifier.
    pub task_id: String,
    /// Tuple payload (JSON string or K-DSL wire string).
    pub payload: String,
    /// RFC3339 creation timestamp.
    pub created_at: String,
}

/// Structured tuple payload emitted upon verification completion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationTuplePayload {
    /// Associated task identifier.
    pub task_id: String,
    /// High-level outcome status (`PASS`, `FAILED`, `BLOCKED`).
    pub status: String,
    /// Explicit receipt verdict status.
    pub verdict: VerdictStatus,
    /// Absolute or relative path to the persisted signed execution receipt.
    pub receipt_path: PathBuf,
    /// Cryptographic SHA-256 fingerprint of the verified git patch diff.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_patch_sha256: Option<String>,
    /// Diagnostic failure reason if verification did not succeed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
}

/// Resolves the Linda Tuple Space directory following the standard precedence:
/// 1. Explicit override path.
/// 2. `K0MARU_TUPLE_DIR` environment variable.
/// 3. Default `~/.k0maru/tuples`.
pub fn resolve_tuples_dir(override_dir: Option<&Path>) -> PathBuf {
    if let Some(dir) = override_dir {
        return dir.to_path_buf();
    }

    if let Ok(env_path) = std::env::var(ENV_K0MARU_TUPLE_DIR) {
        let trimmed = env_path.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));

    home.join(DEFAULT_K0MARU_DIR).join(DEFAULT_TUPLES_SUBDIR)
}

/// Emitter for publishing verification outcome tuples into Linda Tuple Space.
pub struct TupleEmitter;

impl TupleEmitter {
    /// Emits a generic Linda tuple atomically to disk.
    pub fn emit(
        tuple_dir: Option<&Path>,
        kind: &str,
        domain: &str,
        task_id: &str,
        payload: &str,
    ) -> Result<LindaTuple, VerifierError> {
        let dir = resolve_tuples_dir(tuple_dir);
        std::fs::create_dir_all(&dir)?;

        let tuple_id = format!("tup_{}", Uuid::new_v4().simple());
        let tuple = LindaTuple {
            id: tuple_id.clone(),
            kind: kind.to_string(),
            domain: domain.to_string(),
            task_id: task_id.to_string(),
            payload: payload.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };

        let json = serde_json::to_string_pretty(&tuple).map_err(|e| {
            VerifierError::InvalidReceipt(format!("Tuple serialization failed: {e}"))
        })?;

        let target_path = dir.join(format!("{tuple_id}.json"));
        let temp_path = dir.join(format!(".tmp_{}_{}", tuple_id, std::process::id()));

        // Atomic write via temporary file
        std::fs::write(&temp_path, json.as_bytes())?;
        std::fs::rename(&temp_path, &target_path)?;

        Ok(tuple)
    }

    /// Emits a typed verification outcome tuple (`TASK_VERIFIED` on Success, `TASK_BLOCKED` on Failed/Blocked).
    pub fn emit_verification_outcome(
        tuple_dir: Option<&Path>,
        domain: Option<&str>,
        receipt: &ExecutionReceipt,
        receipt_path: &Path,
    ) -> Result<LindaTuple, VerifierError> {
        let active_domain = domain.unwrap_or(DEFAULT_TUPLE_DOMAIN);

        let (kind, status, failure_reason) = match receipt.verdict {
            VerdictStatus::Success => (TUPLE_KIND_VERIFIED, "PASS", None),
            VerdictStatus::Failed => {
                let reason = format_failure_reason(receipt);
                (TUPLE_KIND_BLOCKED, "FAILED", Some(reason))
            }
            VerdictStatus::Blocked => {
                let reason = format_failure_reason(receipt);
                (TUPLE_KIND_BLOCKED, "BLOCKED", Some(reason))
            }
        };

        let payload_struct = VerificationTuplePayload {
            task_id: receipt.task_id.clone(),
            status: status.to_string(),
            verdict: receipt.verdict,
            receipt_path: receipt_path.to_path_buf(),
            git_patch_sha256: receipt.artifacts.git_patch_sha256.clone(),
            failure_reason,
        };

        let payload_json = serde_json::to_string(&payload_struct).map_err(|e| {
            VerifierError::InvalidReceipt(format!("Payload serialization failed: {e}"))
        })?;

        Self::emit(
            tuple_dir,
            kind,
            active_domain,
            &receipt.task_id,
            &payload_json,
        )
    }
}

fn format_failure_reason(receipt: &ExecutionReceipt) -> String {
    if receipt.checks.scope_guard.status == StepStatus::Failed {
        if let Some(ref msg) = receipt.checks.scope_guard.message {
            return msg.clone();
        }
        if !receipt.checks.scope_guard.unauthorized_files.is_empty() {
            return format!(
                "Scope violation: unauthorized files modified ({})",
                receipt.checks.scope_guard.unauthorized_files.join(", ")
            );
        }
        return "Scope guard check failed".to_string();
    }

    if receipt.checks.compilation.status == StepStatus::Failed {
        return format!(
            "Compilation failed with exit code {}",
            receipt.checks.compilation.exit_code
        );
    }

    if receipt.checks.tests.status == StepStatus::Failed {
        return format!(
            "Test suite failed: {}/{} tests failed",
            receipt.checks.tests.failed, receipt.checks.tests.total
        );
    }

    if receipt.checks.linter.status == StepStatus::Failed {
        return format!(
            "Linter failed with {} warnings",
            receipt.checks.linter.warnings
        );
    }

    if receipt.verdict == VerdictStatus::Blocked {
        return "Verification pipeline blocked: environment or runner error".to_string();
    }

    "Verification checks failed".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::receipt::{
        ArtifactSummary, CheckSummary, CompilationCheck, LinterCheck, ScopeGuardCheck, TestCheck,
    };
    use tempfile::TempDir;

    fn sample_receipt(verdict: VerdictStatus) -> ExecutionReceipt {
        ExecutionReceipt {
            schema: crate::constants::DEFAULT_RECEIPT_SCHEMA.to_string(),
            task_id: "test-task-tuple-01".to_string(),
            workspace: PathBuf::from("/test/workspace"),
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
                    total: 10,
                    failed: 0,
                    duration_ms: Some(150),
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
                    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string(),
                ),
                receipt_timestamp: Some("2026-10-13T12:00:00Z".to_string()),
            },
        }
    }

    #[test]
    fn test_resolve_tuples_dir_hierarchy() {
        let custom = Path::new("/custom/tuples");
        assert_eq!(
            resolve_tuples_dir(Some(custom)),
            PathBuf::from("/custom/tuples")
        );

        std::env::set_var(ENV_K0MARU_TUPLE_DIR, "/env/tuples");
        assert_eq!(resolve_tuples_dir(None), PathBuf::from("/env/tuples"));

        std::env::remove_var(ENV_K0MARU_TUPLE_DIR);
        let default_dir = resolve_tuples_dir(None);
        assert!(
            default_dir.ends_with(".k0maru/tuples") || default_dir.ends_with(".k0maru\\tuples")
        );
    }

    #[test]
    fn test_emit_verification_outcome_success() {
        let temp = TempDir::new().unwrap();
        let receipt = sample_receipt(VerdictStatus::Success);
        let receipt_path = Path::new("/path/to/receipt.json");

        let tuple = TupleEmitter::emit_verification_outcome(
            Some(temp.path()),
            Some("AUTH-SRV"),
            &receipt,
            receipt_path,
        )
        .expect("emit verified tuple");

        assert_eq!(tuple.kind, TUPLE_KIND_VERIFIED);
        assert_eq!(tuple.domain, "AUTH-SRV");
        assert_eq!(tuple.task_id, "test-task-tuple-01");
        assert!(tuple.id.starts_with("tup_"));

        let payload: VerificationTuplePayload =
            serde_json::from_str(&tuple.payload).expect("parse payload JSON");
        assert_eq!(payload.status, "PASS");
        assert_eq!(payload.verdict, VerdictStatus::Success);
        assert_eq!(payload.receipt_path, PathBuf::from("/path/to/receipt.json"));
        assert_eq!(
            payload.git_patch_sha256.as_deref(),
            Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
        );
        assert_eq!(payload.failure_reason, None);

        // Verify physical file was written to tuple directory
        let written_file = temp.path().join(format!("{}.json", tuple.id));
        assert!(written_file.is_file());
    }

    #[test]
    fn test_emit_verification_outcome_blocked() {
        let temp = TempDir::new().unwrap();
        let receipt = sample_receipt(VerdictStatus::Failed);
        let receipt_path = Path::new("/path/to/failed_receipt.json");

        let tuple = TupleEmitter::emit_verification_outcome(
            Some(temp.path()),
            None,
            &receipt,
            receipt_path,
        )
        .expect("emit blocked tuple");

        assert_eq!(tuple.kind, TUPLE_KIND_BLOCKED);
        assert_eq!(tuple.domain, DEFAULT_TUPLE_DOMAIN);
        assert_eq!(tuple.task_id, "test-task-tuple-01");

        let payload: VerificationTuplePayload =
            serde_json::from_str(&tuple.payload).expect("parse payload JSON");
        assert_eq!(payload.status, "FAILED");
        assert_eq!(payload.verdict, VerdictStatus::Failed);
        assert!(payload.failure_reason.is_some());
        assert!(payload
            .failure_reason
            .unwrap()
            .contains("Compilation failed"));
        assert_eq!(
            payload.receipt_path,
            PathBuf::from("/path/to/failed_receipt.json")
        );
    }
}
