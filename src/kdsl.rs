//! K-DSL 0.1 expression parser for deterministic verification directives.
//!
//! K-DSL (K0maru Machine-Native Domain-Specific Language) expresses dynamic
//! verification invariants via the `!Σ:VERIFY(...)` syntax.
//!
//! # Examples:
//! ```text
//! !Σ:VERIFY(COMPILE,TEST,SCOPE,LIVING_ADR#SEC-004)
//! !Σ:VERIFY(CARGO_CHECK,IMMUTABLE_TESTS#main)
//! !δ:0.1|AUTH-SRV|src/auth.ts#v1 §AST:F[verify] @OP:... !Σ:VERIFY(TS_COMPILE,LIVING_ADR#SEC-004)
//! ```

use crate::error::VerifierError;
use serde::{Deserialize, Serialize};

/// High-level verification invariant requested in a K-DSL expression.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum KdslInvariant {
    /// Compilation / static type check (e.g. `COMPILE`, `CARGO_CHECK`, `TS_COMPILE`).
    Compile,
    /// Automated test suite execution (e.g. `TEST`, `CARGO_TEST`, `PYTEST`).
    Test,
    /// Scope guard and whitelist compliance check (e.g. `SCOPE`, `SCOPE_GUARD`).
    Scope,
    /// Living ADR architectural invariant check with optional rule ID (e.g. `LIVING_ADR#SEC-004`).
    LivingAdr(Option<String>),
    /// Immutable test baseline guard with optional base branch (e.g. `IMMUTABLE_TESTS#main`, `NO_DELETIONS`).
    ImmutableTests(Option<String>),
    /// Popperian physical falsification check ensuring tests fail on baseline (e.g. `RED_CHECK`, `FALSIFICATION`).
    RedCheck,
    /// Custom or extension invariant.
    Custom(String),
}

/// Parsed K-DSL verification directive clause.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KdslVerifyClause {
    /// Raw verification expression string.
    pub raw: String,
    /// Optional domain extracted from full K-DSL header (e.g. `AUTH-SRV`).
    pub domain: Option<String>,
    /// List of parsed invariants.
    pub invariants: Vec<KdslInvariant>,
}

impl KdslVerifyClause {
    /// Parses a K-DSL wire line, a `!Σ:VERIFY(...)` expression, or a comma-separated invariant list.
    pub fn parse(input: &str) -> Result<Self, VerifierError> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(VerifierError::InvalidReceipt(
                "K-DSL expression cannot be empty".to_string(),
            ));
        }

        // 1. Extract domain if input is a full K-DSL envelope line (!δ:<version>|<domain>|...)
        let domain = if trimmed.starts_with("!δ:") {
            trimmed
                .split('|')
                .nth(1)
                .map(|d| d.trim().to_string())
                .filter(|d| !d.is_empty())
        } else {
            None
        };

        // 2. Locate the !Σ:VERIFY(...) or VERIFY(...) clause
        let verify_content = if let Some(idx) = trimmed.find("!Σ:VERIFY(") {
            let start = idx + "!Σ:VERIFY(".len();
            let end = trimmed[start..].find(')').ok_or_else(|| {
                VerifierError::InvalidReceipt("Unclosed '!Σ:VERIFY(' in K-DSL".to_string())
            })?;
            &trimmed[start..start + end]
        } else if let Some(idx) = trimmed.find("VERIFY(") {
            let start = idx + "VERIFY(".len();
            let end = trimmed[start..].find(')').ok_or_else(|| {
                VerifierError::InvalidReceipt("Unclosed 'VERIFY(' in expression".to_string())
            })?;
            &trimmed[start..start + end]
        } else {
            trimmed
        };

        // 3. Parse individual comma-separated invariants
        let mut invariants = Vec::new();
        for item in verify_content.split(',') {
            let token = item.trim();
            if token.is_empty() {
                continue;
            }

            let invariant = Self::parse_single_invariant(token);
            invariants.push(invariant);
        }

        if invariants.is_empty() {
            return Err(VerifierError::InvalidReceipt(format!(
                "No valid verification invariants found in K-DSL expression: '{input}'"
            )));
        }

        Ok(Self {
            raw: trimmed.to_string(),
            domain,
            invariants,
        })
    }

    fn parse_single_invariant(token: &str) -> KdslInvariant {
        let upper = token.to_ascii_uppercase();

        if upper == "COMPILE" || upper == "CARGO_CHECK" || upper == "TS_COMPILE" || upper == "CHECK"
        {
            KdslInvariant::Compile
        } else if upper == "TEST"
            || upper == "CARGO_TEST"
            || upper == "PYTEST"
            || upper == "ALL_TESTS"
        {
            KdslInvariant::Test
        } else if upper == "SCOPE" || upper == "SCOPE_GUARD" {
            KdslInvariant::Scope
        } else if upper.starts_with("LIVING_ADR") {
            let rule = if let Some((_, rule_id)) = token.split_once('#') {
                let trimmed_rule = rule_id.trim();
                if trimmed_rule.is_empty() {
                    None
                } else {
                    Some(trimmed_rule.to_string())
                }
            } else {
                None
            };
            KdslInvariant::LivingAdr(rule)
        } else if upper.starts_with("IMMUTABLE_TESTS")
            || upper == "NO_DELETIONS"
            || upper.starts_with("BASELINE")
        {
            let branch = if let Some((_, b)) = token.split_once('#') {
                let trimmed_branch = b.trim();
                if trimmed_branch.is_empty() {
                    None
                } else {
                    Some(trimmed_branch.to_string())
                }
            } else {
                None
            };
            KdslInvariant::ImmutableTests(branch)
        } else if upper == "RED_CHECK"
            || upper == "RED_TEST"
            || upper == "FALSIFICATION"
            || upper == "POPPER"
            || upper == "POPPERIAN"
        {
            KdslInvariant::RedCheck
        } else {
            KdslInvariant::Custom(token.to_string())
        }
    }

    /// Whether compilation check is required.
    pub fn has_compile(&self) -> bool {
        self.invariants
            .iter()
            .any(|i| matches!(i, KdslInvariant::Compile))
    }

    /// Whether test suite execution is required.
    pub fn has_test(&self) -> bool {
        self.invariants
            .iter()
            .any(|i| matches!(i, KdslInvariant::Test))
    }

    /// Whether scope guard check is required.
    pub fn has_scope(&self) -> bool {
        self.invariants
            .iter()
            .any(|i| matches!(i, KdslInvariant::Scope))
    }

    /// Whether living ADR check is requested.
    pub fn has_living_adr(&self) -> bool {
        self.invariants
            .iter()
            .any(|i| matches!(i, KdslInvariant::LivingAdr(_)))
    }

    /// Returns target ADR rule ID if explicitly parameterized.
    pub fn living_adr_target(&self) -> Option<&str> {
        self.invariants.iter().find_map(|i| match i {
            KdslInvariant::LivingAdr(Some(ref rule)) => Some(rule.as_str()),
            _ => None,
        })
    }

    /// Whether immutable baseline test protection is required.
    pub fn has_immutable_tests(&self) -> bool {
        self.invariants
            .iter()
            .any(|i| matches!(i, KdslInvariant::ImmutableTests(_)))
    }

    /// Returns explicit base branch for immutable test check if parameterized.
    pub fn immutable_base_branch(&self) -> Option<&str> {
        self.invariants.iter().find_map(|i| match i {
            KdslInvariant::ImmutableTests(Some(ref b)) => Some(b.as_str()),
            _ => None,
        })
    }

    /// Whether Popperian physical falsification check is required.
    pub fn has_red_check(&self) -> bool {
        self.invariants
            .iter()
            .any(|i| matches!(i, KdslInvariant::RedCheck))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_direct_verify_clause() {
        let input = "!Σ:VERIFY(COMPILE,TEST,SCOPE,LIVING_ADR#SEC-004)";
        let clause = KdslVerifyClause::parse(input).expect("parse kdsl");

        assert_eq!(clause.invariants.len(), 4);
        assert!(clause.has_compile());
        assert!(clause.has_test());
        assert!(clause.has_scope());
        assert!(clause.has_living_adr());
        assert_eq!(clause.living_adr_target(), Some("SEC-004"));
        assert!(!clause.has_immutable_tests());
    }

    #[test]
    fn test_parse_full_kdsl_wire_line() {
        let line = "!δ:0.1|AUTH-SRV|src/middleware/auth.ts#v4182 §AST:F[verifyToken] @OP:SWAP_ID (a) -> (b) !Σ:VERIFY(TS_COMPILE,LIVING_ADR#SEC-004,IMMUTABLE_TESTS#main)";
        let clause = KdslVerifyClause::parse(line).expect("parse full wire");

        assert_eq!(clause.domain.as_deref(), Some("AUTH-SRV"));
        assert!(clause.has_compile());
        assert!(!clause.has_test());
        assert!(clause.has_living_adr());
        assert_eq!(clause.living_adr_target(), Some("SEC-004"));
        assert!(clause.has_immutable_tests());
        assert_eq!(clause.immutable_base_branch(), Some("main"));
    }

    #[test]
    fn test_parse_comma_separated_aliases() {
        let input = "CARGO_CHECK, CARGO_TEST, SCOPE_GUARD, NO_DELETIONS";
        let clause = KdslVerifyClause::parse(input).expect("parse aliases");

        assert!(clause.has_compile());
        assert!(clause.has_test());
        assert!(clause.has_scope());
        assert!(clause.has_immutable_tests());
    }

    #[test]
    fn test_parse_living_adr_without_id() {
        let input = "!Σ:VERIFY(LIVING_ADR)";
        let clause = KdslVerifyClause::parse(input).expect("parse living adr");
        assert!(clause.has_living_adr());
        assert_eq!(clause.living_adr_target(), None);
    }

    #[test]
    fn test_parse_custom_invariants() {
        let input = "VERIFY(COMPILE, CUSTOM_STATIC_ANALYSIS, PERFORMANCE_BENCHMARK)";
        let clause = KdslVerifyClause::parse(input).expect("parse custom");

        assert!(clause.has_compile());
        assert_eq!(
            clause.invariants[1],
            KdslInvariant::Custom("CUSTOM_STATIC_ANALYSIS".to_string())
        );
        assert_eq!(
            clause.invariants[2],
            KdslInvariant::Custom("PERFORMANCE_BENCHMARK".to_string())
        );
    }

    #[test]
    fn test_parse_empty_errors() {
        assert!(KdslVerifyClause::parse("   ").is_err());
        assert!(KdslVerifyClause::parse("!Σ:VERIFY()").is_err());
        assert!(KdslVerifyClause::parse("!Σ:VERIFY(   )").is_err());
    }
}
