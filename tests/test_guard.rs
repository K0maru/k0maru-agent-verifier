use k0maru_verifier::guard::{
    evaluate_scope, matches_pattern, normalize_path, parse_porcelain_line, ScopeGuard,
};
use k0maru_verifier::receipt::StepStatus;
use std::path::Path;
use tempfile::TempDir;

/// Helper to set up an initialized git repository with an initial commit.
fn setup_temp_git_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let run_git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .expect("execute git command");
        assert!(
            output.status.success(),
            "Git command failed: {:?}, stderr: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    };

    run_git(&["init"]);
    run_git(&["config", "user.name", "K0maru Guard Tester"]);
    run_git(&["config", "user.email", "guard@k0maru.dev"]);
    run_git(&["config", "commit.gpgsign", "false"]);

    // Initial commit
    std::fs::write(root.join("README.md"), "# Project").unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/lib.rs"), "pub fn hello() {}\n").unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"demo\"\n").unwrap();

    run_git(&["add", "."]);
    run_git(&["commit", "-m", "Initial commit"]);

    temp
}

fn run_git_in(dir: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("execute git");
    assert!(
        output.status.success(),
        "Git failed: {:?}, stderr: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

// ============================================================================
// 1. Path Normalization Tests
// ============================================================================

#[test]
fn test_normalize_path_unix_and_windows() {
    // Basic unix
    assert_eq!(normalize_path("src/lib.rs"), "src/lib.rs");
    assert_eq!(normalize_path("./src/lib.rs"), "src/lib.rs");
    assert_eq!(normalize_path("./././src/lib.rs"), "src/lib.rs");
    assert_eq!(normalize_path("/src/lib.rs"), "src/lib.rs");

    // Windows backslashes
    assert_eq!(normalize_path("src\\lib.rs"), "src/lib.rs");
    assert_eq!(
        normalize_path(".\\src\\nested\\mod.rs"),
        "src/nested/mod.rs"
    );
    assert_eq!(
        normalize_path("src\\nested\\deep\\file.rs"),
        "src/nested/deep/file.rs"
    );

    // Quotes and whitespace
    assert_eq!(normalize_path("  \"src/lib.rs\"  "), "src/lib.rs");
    assert_eq!(normalize_path("  'src/lib.rs'  "), "src/lib.rs");
    assert_eq!(normalize_path("\"./src/main.rs\""), "src/main.rs");

    // Empty and whitespace only
    assert_eq!(normalize_path(""), "");
    assert_eq!(normalize_path("   "), "");
    assert_eq!(normalize_path("./"), "");
}

// ============================================================================
// 2. Pattern Matching Semantics Tests
// ============================================================================

#[test]
fn test_matches_pattern_exact_matching() {
    assert!(matches_pattern("Cargo.toml", "Cargo.toml"));
    assert!(matches_pattern("./Cargo.toml", "Cargo.toml"));
    assert!(matches_pattern("Cargo.toml", "./Cargo.toml"));
    assert!(matches_pattern("src/lib.rs", "src/lib.rs"));
    assert!(matches_pattern(".\\src\\lib.rs", "src/lib.rs"));

    assert!(!matches_pattern("Cargo.toml", "Cargo.lock"));
    assert!(!matches_pattern("src/lib.rs", "src/main.rs"));
}

#[test]
fn test_matches_pattern_directory_prefix() {
    // With trailing slash
    assert!(matches_pattern("src/", "src/lib.rs"));
    assert!(matches_pattern("src/", "src/module/sub.rs"));
    assert!(matches_pattern("./src/", "src/lib.rs"));
    assert!(!matches_pattern("src/", "tests/test.rs"));
    assert!(!matches_pattern("src/", "src_other/lib.rs"));

    // Without trailing slash (should match dir contents, but NOT prefix names like src_old)
    assert!(matches_pattern("src", "src/lib.rs"));
    assert!(matches_pattern("src", "src/nested/deep.rs"));
    assert!(!matches_pattern("src", "src_backup/lib.rs"));
    assert!(!matches_pattern("src", "source/lib.rs"));
}

#[test]
fn test_matches_pattern_wildcards() {
    // Extension wildcard: *.rs matches anywhere in path hierarchy
    assert!(matches_pattern("*.rs", "lib.rs"));
    assert!(matches_pattern("*.rs", "src/lib.rs"));
    assert!(matches_pattern("*.rs", "src/nested/mod.rs"));
    assert!(!matches_pattern("*.rs", "Cargo.toml"));
    assert!(!matches_pattern("*.rs", "src/lib.rs.bak"));

    // Directory-scoped wildcard: src/*.rs (single level)
    assert!(matches_pattern("src/*.rs", "src/lib.rs"));
    assert!(matches_pattern("src/*.rs", "src/main.rs"));
    assert!(!matches_pattern("src/*.rs", "src/nested/mod.rs"));
    assert!(!matches_pattern("src/*.rs", "tests/lib.rs"));

    // Recursive wildcard: src/**/*.rs
    assert!(matches_pattern("src/**/*.rs", "src/lib.rs"));
    assert!(matches_pattern("src/**/*.rs", "src/nested/mod.rs"));
    assert!(matches_pattern("src/**/*.rs", "src/a/b/c/d.rs"));
    assert!(!matches_pattern("src/**/*.rs", "tests/a/b.rs"));

    // Single character wildcard: ?
    assert!(matches_pattern("test_?.rs", "test_1.rs"));
    assert!(matches_pattern("test_?.rs", "test_a.rs"));
    assert!(!matches_pattern("test_?.rs", "test_12.rs"));
}

// ============================================================================
// 3. Porcelain Line Parsing Tests
// ============================================================================

#[test]
fn test_parse_porcelain_line_variations() {
    assert_eq!(
        parse_porcelain_line("?? untracked.rs"),
        vec!["untracked.rs"]
    );
    assert_eq!(parse_porcelain_line(" M src/lib.rs"), vec!["src/lib.rs"]);
    assert_eq!(parse_porcelain_line("M  src/main.rs"), vec!["src/main.rs"]);
    assert_eq!(
        parse_porcelain_line("A  new_file.txt"),
        vec!["new_file.txt"]
    );
    assert_eq!(parse_porcelain_line("D  deleted.txt"), vec!["deleted.txt"]);
    assert_eq!(
        parse_porcelain_line("R  old_name.rs -> new_name.rs"),
        vec!["old_name.rs", "new_name.rs"]
    );
    assert_eq!(
        parse_porcelain_line("?? \"quoted with spaces.rs\""),
        vec!["quoted with spaces.rs"]
    );

    // Ignored files (!!) should be skipped
    assert!(parse_porcelain_line("!! target/").is_empty());
    assert!(parse_porcelain_line("!! .env").is_empty());

    // Empty or corrupted line
    assert!(parse_porcelain_line("").is_empty());
    assert!(parse_porcelain_line("  ").is_empty());
}

// ============================================================================
// 4. Pure Scope Evaluation Tests (evaluate_scope)
// ============================================================================

#[test]
fn test_evaluate_scope_empty_or_no_whitelist() {
    let modified = vec!["src/lib.rs".to_string(), "Cargo.toml".to_string()];

    // None whitelist -> Passed
    let res1 = evaluate_scope(&modified, None);
    assert_eq!(res1.status, StepStatus::Passed);
    assert!(res1.is_passed());
    assert_eq!(res1.modified_files, vec!["Cargo.toml", "src/lib.rs"]);
    assert!(res1.unauthorized_files.is_empty());

    // Empty slice whitelist -> Passed
    let empty_list: Vec<String> = Vec::new();
    let res2 = evaluate_scope(&modified, Some(&empty_list));
    assert_eq!(res2.status, StepStatus::Passed);
    assert!(res2.is_passed());
    assert!(res2.unauthorized_files.is_empty());
}

#[test]
fn test_evaluate_scope_with_whitelist_and_unauthorized_detection() {
    let modified = vec![
        "src/lib.rs".to_string(),
        "tests/integration.rs".to_string(),
        "secret.key".to_string(),
        "config.json".to_string(),
    ];

    let whitelist = vec!["src/".to_string(), "tests/*.rs".to_string()];
    let res = evaluate_scope(&modified, Some(&whitelist));

    assert_eq!(res.status, StepStatus::Failed);
    assert!(!res.is_passed());
    assert_eq!(res.unauthorized_files, vec!["config.json", "secret.key"]);
    assert_eq!(res.modified_files.len(), 4);
}

#[test]
fn test_evaluate_scope_windows_path_normalization() {
    let modified = vec![
        ".\\src\\lib.rs".to_string(),
        "src\\nested\\mod.rs".to_string(),
    ];
    let whitelist = vec!["src/".to_string()];

    let res = evaluate_scope(&modified, Some(&whitelist));
    assert_eq!(res.status, StepStatus::Passed);
    assert!(res.is_passed());
    assert_eq!(res.modified_files, vec!["src/lib.rs", "src/nested/mod.rs"]);
    assert!(res.unauthorized_files.is_empty());
}

// ============================================================================
// 5. End-to-End ScopeGuard in Real Git Repositories
// ============================================================================

#[tokio::test]
async fn test_git_repo_clean_state() {
    let repo = setup_temp_git_repo();
    let root = repo.path();

    // Check with whitelist
    let whitelist = vec!["src/".to_string(), "Cargo.toml".to_string()];
    let check = ScopeGuard::check_scope(root, Some(&whitelist))
        .await
        .expect("check_scope succeeds");

    assert_eq!(check.status, StepStatus::Passed);
    assert!(check.is_passed());
    assert!(check.modified_files.is_empty());
    assert!(check.unauthorized_files.is_empty());
}

#[tokio::test]
async fn test_git_repo_modified_tracked_files_within_whitelist() {
    let repo = setup_temp_git_repo();
    let root = repo.path();

    // Modify src/lib.rs (tracked)
    std::fs::write(root.join("src/lib.rs"), "pub fn modified() {}\n").unwrap();

    let whitelist = vec!["src/".to_string()];
    let check = ScopeGuard::check_scope(root, Some(&whitelist))
        .await
        .expect("check_scope succeeds");

    assert_eq!(check.status, StepStatus::Passed);
    assert!(check.is_passed());
    assert_eq!(check.modified_files, vec!["src/lib.rs"]);
    assert!(check.unauthorized_files.is_empty());
}

#[tokio::test]
async fn test_git_repo_modified_tracked_files_outside_whitelist() {
    let repo = setup_temp_git_repo();
    let root = repo.path();

    // Modify README.md (tracked) and src/lib.rs (tracked)
    std::fs::write(root.join("src/lib.rs"), "pub fn updated() {}\n").unwrap();
    std::fs::write(root.join("README.md"), "# Updated README\n").unwrap();

    // Whitelist only allows src/
    let whitelist = vec!["src/".to_string()];
    let check = ScopeGuard::check_scope(root, Some(&whitelist))
        .await
        .expect("check_scope succeeds");

    assert_eq!(check.status, StepStatus::Failed);
    assert!(!check.is_passed());
    assert_eq!(check.unauthorized_files, vec!["README.md"]);
    assert_eq!(check.modified_files, vec!["README.md", "src/lib.rs"]);
}

#[tokio::test]
async fn test_git_repo_newly_created_untracked_files_outside_whitelist() {
    let repo = setup_temp_git_repo();
    let root = repo.path();

    // Create untracked file outside whitelist
    std::fs::write(root.join("untracked_leak.env"), "SECRET=xyz\n").unwrap();

    let whitelist = vec!["src/".to_string()];
    let check = ScopeGuard::check_scope(root, Some(&whitelist))
        .await
        .expect("check_scope succeeds");

    assert_eq!(check.status, StepStatus::Failed);
    assert!(!check.is_passed());
    assert_eq!(check.unauthorized_files, vec!["untracked_leak.env"]);
    assert_eq!(check.modified_files, vec!["untracked_leak.env"]);
}

#[tokio::test]
async fn test_git_repo_newly_created_untracked_files_inside_whitelist() {
    let repo = setup_temp_git_repo();
    let root = repo.path();

    // Create untracked file inside allowed src/ directory
    std::fs::write(root.join("src/new_mod.rs"), "pub mod helper;\n").unwrap();

    let whitelist = vec!["src/".to_string()];
    let check = ScopeGuard::check_scope(root, Some(&whitelist))
        .await
        .expect("check_scope succeeds");

    assert_eq!(check.status, StepStatus::Passed);
    assert!(check.is_passed());
    assert_eq!(check.modified_files, vec!["src/new_mod.rs"]);
    assert!(check.unauthorized_files.is_empty());
}

#[tokio::test]
async fn test_git_repo_staged_changes() {
    let repo = setup_temp_git_repo();
    let root = repo.path();

    // Modify and stage src/lib.rs
    std::fs::write(root.join("src/lib.rs"), "pub fn staged_lib() {}\n").unwrap();
    run_git_in(root, &["add", "src/lib.rs"]);

    // Create unstaged file
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"demo2\"\n").unwrap();

    let whitelist = vec!["src/".to_string(), "Cargo.toml".to_string()];
    let check = ScopeGuard::check_scope(root, Some(&whitelist))
        .await
        .expect("check_scope succeeds");

    assert_eq!(check.status, StepStatus::Passed);
    assert!(check.is_passed());
    assert_eq!(check.modified_files, vec!["Cargo.toml", "src/lib.rs"]);
    assert!(check.unauthorized_files.is_empty());
}

#[tokio::test]
async fn test_git_repo_deleted_file() {
    let repo = setup_temp_git_repo();
    let root = repo.path();

    // Delete Cargo.toml
    std::fs::remove_file(root.join("Cargo.toml")).unwrap();

    // If Cargo.toml is in whitelist, deletion should be authorized
    let whitelist = vec!["Cargo.toml".to_string()];
    let check = ScopeGuard::check_scope(root, Some(&whitelist))
        .await
        .expect("check_scope succeeds");

    assert_eq!(check.status, StepStatus::Passed);
    assert!(check.is_passed());
    assert_eq!(check.modified_files, vec!["Cargo.toml"]);
    assert!(check.unauthorized_files.is_empty());

    // If Cargo.toml is NOT in whitelist, deletion should be unauthorized
    let whitelist_src_only = vec!["src/".to_string()];
    let check_unauth = ScopeGuard::check_scope(root, Some(&whitelist_src_only))
        .await
        .expect("check_scope succeeds");

    assert_eq!(check_unauth.status, StepStatus::Failed);
    assert!(!check_unauth.is_passed());
    assert_eq!(check_unauth.unauthorized_files, vec!["Cargo.toml"]);
}
