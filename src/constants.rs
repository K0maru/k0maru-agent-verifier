//! Typed constants and configuration invariants for k0maru-verifier.

/// Standard process exit codes.
pub const EXIT_SUCCESS: i32 = 0;
pub const EXIT_VERIFICATION_FAILED: i32 = 1;
pub const EXIT_FATAL_ERROR: i32 = 2;

/// Standard environment variable names.
pub const ENV_K0MARU_RECEIPT_DIR: &str = "K0MARU_RECEIPT_DIR";

/// Default file and directory locations.
pub const DEFAULT_K0MARU_DIR: &str = ".k0maru";
pub const DEFAULT_RECEIPTS_SUBDIR: &str = "receipts";
pub const DEFAULT_RECEIPT_SCHEMA: &str = "https://k0maru.dev/schemas/agent-receipt-v1.json";

/// Output buffer and truncation limits (64 KB).
pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 65_536;

/// Process execution timeout in seconds (5 minutes).
pub const DEFAULT_COMMAND_TIMEOUT_SECS: u64 = 300;

/// Default git baseline branches.
pub const DEFAULT_BASE_BRANCH_MAIN: &str = "main";
pub const DEFAULT_BASE_BRANCH_DEV: &str = "dev";

/// Invariant anti-tampering failure message.
pub const ANTI_TAMPERING_VIOLATION_MESSAGE: &str =
    "Anti-tampering violation: deletions detected in existing test baseline";
