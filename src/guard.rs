use crate::error::VerifierError;
use crate::receipt::{ScopeGuardCheck, StepStatus};
use regex::Regex;
use std::collections::BTreeSet;
use std::path::Path;
use tokio::process::Command;

/// Normalizes file paths for deterministic whitelist evaluation.
///
/// Strips leading `./` or `/`, converts backslashes to forward slashes,
/// removes wrapping quotes, and trims whitespace.
pub fn normalize_path(path: &str) -> String {
    let mut s = path.trim();
    if s.len() >= 2
        && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')))
    {
        s = &s[1..s.len() - 1];
    }
    let s = s.trim();
    let replaced = s.replace('\\', "/");

    let mut trimmed = replaced.as_str();
    loop {
        if let Some(rest) = trimmed.strip_prefix("./") {
            trimmed = rest;
        } else if let Some(rest) = trimmed.strip_prefix('/') {
            trimmed = rest;
        } else {
            break;
        }
    }
    trimmed.to_string()
}

/// Converts a glob pattern with `*`, `?`, or `**` to an anchored regular expression.
pub fn glob_to_regex(pattern: &str) -> Option<Regex> {
    let mut regex_str = String::from("^");
    let has_slash = pattern.contains('/');
    if !has_slash {
        // If pattern contains no slash, match file name anywhere in hierarchy (e.g. *.rs)
        regex_str.push_str("(?:.*/)?");
    }

    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' => {
                if chars.peek() == Some(&'*') {
                    chars.next(); // consume second '*'
                    if chars.peek() == Some(&'/') {
                        chars.next(); // consume '/'
                        regex_str.push_str("(?:.*/)?");
                    } else {
                        regex_str.push_str(".*");
                    }
                } else {
                    // Single '*' matches within a single path segment
                    regex_str.push_str("[^/]*");
                }
            }
            '?' => {
                regex_str.push_str("[^/]");
            }
            '.' | '+' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|' | '\\' => {
                regex_str.push('\\');
                regex_str.push(c);
            }
            _ => {
                regex_str.push(c);
            }
        }
    }
    regex_str.push('$');
    Regex::new(&regex_str).ok()
}

/// Evaluates if `file` matches `pattern`.
///
/// Supported matching semantics:
/// 1. Exact match: `pattern == file`
/// 2. Directory prefix: `src/` or `src` matches `src/foo.rs`
/// 3. Wildcard: `*.rs`, `src/*.rs`, `**/*.rs`
pub fn matches_pattern(pattern: &str, file: &str) -> bool {
    let p = normalize_path(pattern);
    let f = normalize_path(file);
    if p.is_empty() || f.is_empty() {
        return false;
    }

    // 1. Exact match
    if p == f {
        return true;
    }

    // 2. Directory prefix match
    let p_clean = p.trim_end_matches('/');
    if f.starts_with(&format!("{p_clean}/")) {
        return true;
    }

    // 3. Wildcard match
    if p.contains('*') || p.contains('?') {
        if let Some(re) = glob_to_regex(&p) {
            if re.is_match(&f) {
                return true;
            }
        }
    }

    false
}

/// Parses a single line from `git status --porcelain` into one or more changed paths.
pub fn parse_porcelain_line(line: &str) -> Vec<String> {
    let trimmed = line.trim_end();
    if trimmed.len() < 3 {
        return Vec::new();
    }

    // Skip ignored files (status !! )
    if trimmed.starts_with("!!") {
        return Vec::new();
    }

    let payload = &trimmed[3..].trim();
    if payload.is_empty() {
        return Vec::new();
    }

    // Handle renames: "orig_path -> new_path"
    if let Some((old_p, new_p)) = payload.split_once(" -> ") {
        let p1 = normalize_path(old_p);
        let p2 = normalize_path(new_p);
        let mut res = Vec::new();
        if !p1.is_empty() {
            res.push(p1);
        }
        if !p2.is_empty() {
            res.push(p2);
        }
        res
    } else {
        let p = normalize_path(payload);
        if !p.is_empty() {
            vec![p]
        } else {
            Vec::new()
        }
    }
}

/// Pure evaluation function comparing a slice of modified files against an optional whitelist.
///
/// If `allowed_whitelist` is `None` or empty, all modifications are authorized.
/// Otherwise, any file not matching the whitelist is flagged as unauthorized.
pub fn evaluate_scope(
    modified_files: &[String],
    allowed_whitelist: Option<&[String]>,
) -> ScopeGuardCheck {
    let mut normalized_set = BTreeSet::new();
    for f in modified_files {
        let norm = normalize_path(f);
        if !norm.is_empty() {
            normalized_set.insert(norm);
        }
    }

    let all_modified: Vec<String> = normalized_set.into_iter().collect();

    let is_whitelist_active = match allowed_whitelist {
        Some(list) => !list.is_empty(),
        None => false,
    };

    if !is_whitelist_active {
        return ScopeGuardCheck {
            status: StepStatus::Passed,
            modified_files: all_modified,
            unauthorized_files: Vec::new(),
        };
    }

    let whitelist = allowed_whitelist.unwrap();
    let mut unauthorized = BTreeSet::new();

    for file in &all_modified {
        let is_allowed = whitelist.iter().any(|pat| matches_pattern(pat, file));
        if !is_allowed {
            unauthorized.insert(file.clone());
        }
    }

    let unauthorized_files: Vec<String> = unauthorized.into_iter().collect();
    let status = if unauthorized_files.is_empty() {
        StepStatus::Passed
    } else {
        StepStatus::Failed
    };

    ScopeGuardCheck {
        status,
        modified_files: all_modified,
        unauthorized_files,
    }
}

/// Collects all modified, staged, unstaged, and untracked files in the workspace.
pub async fn collect_modified_files(workspace: &Path) -> Result<Vec<String>, VerifierError> {
    let mut files = BTreeSet::new();

    // 1. Inspect git status --porcelain -uall (captures untracked and staged/unstaged changes)
    let status_output = Command::new("git")
        .args(["status", "--porcelain", "-uall"])
        .current_dir(workspace)
        .output()
        .await;

    let status_output = match status_output {
        Ok(out) => out,
        Err(e) => {
            return Err(VerifierError::ProcessError(format!(
                "Failed to execute 'git status --porcelain': {e}"
            )));
        }
    };

    if !status_output.status.success() {
        let err_msg = String::from_utf8_lossy(&status_output.stderr)
            .trim()
            .to_string();
        return Err(VerifierError::GitError(format!(
            "git status failed with exit code {:?}: {err_msg}",
            status_output.status.code()
        )));
    }

    let status_text = String::from_utf8_lossy(&status_output.stdout);
    for line in status_text.lines() {
        for path in parse_porcelain_line(line) {
            let normalized = normalize_path(&path);
            if !normalized.is_empty() {
                files.insert(normalized);
            }
        }
    }

    // 2. Inspect git diff --name-only HEAD (captures tracked diffs relative to HEAD)
    let diff_output = Command::new("git")
        .args(["diff", "--name-only", "HEAD"])
        .current_dir(workspace)
        .output()
        .await;

    if let Ok(out) = diff_output {
        if out.status.success() {
            let diff_text = String::from_utf8_lossy(&out.stdout);
            for line in diff_text.lines() {
                let normalized = normalize_path(line);
                if !normalized.is_empty() {
                    files.insert(normalized);
                }
            }
        }
    }

    Ok(files.into_iter().collect())
}

/// Scope guard engine enforcing file change blast radius constraints.
pub struct ScopeGuard;

impl ScopeGuard {
    /// Inspects the workspace repository and checks modified files against the declared whitelist.
    pub async fn check_scope(
        workspace: &Path,
        allowed_whitelist: Option<&[String]>,
    ) -> Result<ScopeGuardCheck, VerifierError> {
        let modified = collect_modified_files(workspace).await?;
        Ok(evaluate_scope(&modified, allowed_whitelist))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_normalize_path_various_formats() {
        assert_eq!(normalize_path("./src/lib.rs"), "src/lib.rs");
        assert_eq!(normalize_path("src/lib.rs"), "src/lib.rs");
        assert_eq!(normalize_path("/src/lib.rs"), "src/lib.rs");
        assert_eq!(normalize_path("src\\nested\\file.rs"), "src/nested/file.rs");
        assert_eq!(normalize_path("\"src/lib.rs\""), "src/lib.rs");
        assert_eq!(normalize_path("'src/lib.rs'"), "src/lib.rs");
        assert_eq!(normalize_path("  ./src/lib.rs  "), "src/lib.rs");
        assert_eq!(normalize_path("././src/lib.rs"), "src/lib.rs");
        assert_eq!(normalize_path(""), "");
    }

    #[test]
    fn test_matches_pattern_exact() {
        assert!(matches_pattern("src/lib.rs", "src/lib.rs"));
        assert!(matches_pattern("./src/lib.rs", "src/lib.rs"));
        assert!(matches_pattern("src/lib.rs", "./src/lib.rs"));
        assert!(!matches_pattern("src/lib.rs", "src/main.rs"));
    }

    #[test]
    fn test_matches_pattern_directory_prefix() {
        // Trailing slash
        assert!(matches_pattern("src/", "src/lib.rs"));
        assert!(matches_pattern("src/", "src/nested/mod.rs"));
        assert!(!matches_pattern("src/", "tests/lib.rs"));

        // Without trailing slash
        assert!(matches_pattern("src", "src/foo.rs"));
        assert!(matches_pattern("src", "src/nested/foo.rs"));
        assert!(!matches_pattern("src", "src_backup/foo.rs"));
        assert!(!matches_pattern("src", "source/foo.rs"));
    }

    #[test]
    fn test_matches_pattern_wildcard() {
        // Simple wildcard extension
        assert!(matches_pattern("*.rs", "lib.rs"));
        assert!(matches_pattern("*.rs", "src/lib.rs"));
        assert!(matches_pattern("*.rs", "src/nested/lib.rs"));
        assert!(!matches_pattern("*.rs", "README.md"));
        assert!(!matches_pattern("*.rs", "src/lib.rs.bak"));

        // Single directory wildcard
        assert!(matches_pattern("src/*.rs", "src/lib.rs"));
        assert!(!matches_pattern("src/*.rs", "src/nested/lib.rs"));
        assert!(!matches_pattern("src/*.rs", "lib.rs"));

        // Recursive wildcard
        assert!(matches_pattern("src/**/*.rs", "src/nested/lib.rs"));
        assert!(matches_pattern("src/**/*.rs", "src/a/b/c.rs"));

        // Question mark wildcard
        assert!(matches_pattern("file?.txt", "file1.txt"));
        assert!(matches_pattern("file?.txt", "fileA.txt"));
        assert!(!matches_pattern("file?.txt", "file12.txt"));
    }

    #[test]
    fn test_parse_porcelain_line() {
        assert_eq!(
            parse_porcelain_line("?? untracked.txt"),
            vec!["untracked.txt"]
        );
        assert_eq!(parse_porcelain_line(" M src/lib.rs"), vec!["src/lib.rs"]);
        assert_eq!(parse_porcelain_line("M  src/main.rs"), vec!["src/main.rs"]);
        assert_eq!(parse_porcelain_line("A  Cargo.toml"), vec!["Cargo.toml"]);
        assert_eq!(
            parse_porcelain_line("D  old_file.txt"),
            vec!["old_file.txt"]
        );
        assert_eq!(
            parse_porcelain_line("R  old.rs -> new.rs"),
            vec!["old.rs", "new.rs"]
        );
        assert_eq!(
            parse_porcelain_line("?? \"quoted path with spaces.txt\""),
            vec!["quoted path with spaces.txt"]
        );
        assert!(parse_porcelain_line("!! ignored.log").is_empty());
        assert!(parse_porcelain_line("").is_empty());
    }

    #[test]
    fn test_evaluate_scope_no_whitelist() {
        let modified = vec!["src/lib.rs".to_string(), "Cargo.toml".to_string()];
        let check = evaluate_scope(&modified, None);
        assert_eq!(check.status, StepStatus::Passed);
        assert!(check.is_passed());
        assert_eq!(check.modified_files, vec!["Cargo.toml", "src/lib.rs"]);
        assert!(check.unauthorized_files.is_empty());
    }

    #[test]
    fn test_evaluate_scope_empty_whitelist() {
        let modified = vec!["src/lib.rs".to_string()];
        let empty_list: Vec<String> = Vec::new();
        let check = evaluate_scope(&modified, Some(&empty_list));
        assert_eq!(check.status, StepStatus::Passed);
        assert!(check.is_passed());
        assert_eq!(check.modified_files, vec!["src/lib.rs"]);
        assert!(check.unauthorized_files.is_empty());
    }

    #[test]
    fn test_evaluate_scope_with_whitelist_all_passed() {
        let modified = vec![
            "src/lib.rs".to_string(),
            "src/guard.rs".to_string(),
            "tests/test.rs".to_string(),
        ];
        let whitelist = vec!["src/".to_string(), "tests/".to_string()];
        let check = evaluate_scope(&modified, Some(&whitelist));
        assert_eq!(check.status, StepStatus::Passed);
        assert!(check.is_passed());
        assert_eq!(check.unauthorized_files.len(), 0);
    }

    #[test]
    fn test_evaluate_scope_with_whitelist_unauthorized_detected() {
        let modified = vec![
            "src/lib.rs".to_string(),
            "secret.env".to_string(),
            "database.sqlite".to_string(),
        ];
        let whitelist = vec!["src/".to_string()];
        let check = evaluate_scope(&modified, Some(&whitelist));
        assert_eq!(check.status, StepStatus::Failed);
        assert!(!check.is_passed());
        assert_eq!(
            check.unauthorized_files,
            vec!["database.sqlite", "secret.env"]
        );
    }

    #[test]
    fn test_evaluate_scope_deduplication_and_sorting() {
        let modified = vec![
            "z_file.rs".to_string(),
            "./a_file.rs".to_string(),
            "a_file.rs".to_string(),
            "m_file.rs".to_string(),
        ];
        let check = evaluate_scope(&modified, None);
        assert_eq!(
            check.modified_files,
            vec!["a_file.rs", "m_file.rs", "z_file.rs"]
        );
    }

    #[tokio::test]
    async fn test_scope_guard_in_real_temp_git_repo() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        // 1. Initialize git repo
        let run_git = |args: &[&str]| {
            let output = std::process::Command::new("git")
                .args(args)
                .current_dir(root)
                .output()
                .expect("git execution");
            assert!(
                output.status.success(),
                "Git command failed: {:?}, stderr: {}",
                args,
                String::from_utf8_lossy(&output.stderr)
            );
        };

        run_git(&["init"]);
        run_git(&["config", "user.name", "Tester"]);
        run_git(&["config", "user.email", "tester@example.com"]);

        // 2. Commit initial file
        std::fs::write(root.join("initial.txt"), "hello").unwrap();
        run_git(&["add", "initial.txt"]);
        run_git(&["commit", "-m", "Initial commit"]);

        // 3. Make changes: modify initial.txt, create untracked extra.rs
        std::fs::write(root.join("initial.txt"), "hello world").unwrap();
        std::fs::create_dir(root.join("src")).unwrap();
        std::fs::write(root.join("src/extra.rs"), "pub fn extra() {}").unwrap();

        // Check scope with whitelist ["src/"] -> initial.txt should be unauthorized
        let check = ScopeGuard::check_scope(root, Some(&["src/".to_string()]))
            .await
            .expect("check_scope");
        assert_eq!(check.status, StepStatus::Failed);
        assert_eq!(check.unauthorized_files, vec!["initial.txt"]);
        assert_eq!(check.modified_files, vec!["initial.txt", "src/extra.rs"]);

        // Check scope with whitelist ["src/", "initial.txt"] -> all authorized
        let check2 =
            ScopeGuard::check_scope(root, Some(&["src/".to_string(), "initial.txt".to_string()]))
                .await
                .expect("check_scope");
        assert_eq!(check2.status, StepStatus::Passed);
        assert!(check2.unauthorized_files.is_empty());
    }
}
