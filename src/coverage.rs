use crate::error::VerifierError;
use crate::guard::{is_test_file, normalize_path};
use crate::receipt::{DiffCoverageCheck, StepStatus};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use tokio::process::Command;

/// Represents added or modified lines within a single file extracted from a git diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiffLines {
    pub path: String,
    pub added_lines: BTreeSet<usize>,
}

/// Parsed code coverage report containing covered line numbers per normalized file path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CoverageReport {
    pub file_coverage: BTreeMap<String, BTreeSet<usize>>,
}

impl CoverageReport {
    pub fn new() -> Self {
        Self {
            file_coverage: BTreeMap::new(),
        }
    }

    /// Adds a covered line number for a specific normalized file path.
    pub fn add_covered_line(&mut self, file_path: &str, line_number: usize) {
        let norm = normalize_path(file_path);
        self.file_coverage
            .entry(norm)
            .or_default()
            .insert(line_number);
    }

    /// Checks if a given line number in a file was covered.
    pub fn is_line_covered(&self, file_path: &str, line_number: usize) -> bool {
        let norm = normalize_path(file_path);
        // Direct match
        if let Some(lines) = self.file_coverage.get(&norm) {
            if lines.contains(&line_number) {
                return true;
            }
        }
        // Suffix / relative match in case coverage report has absolute paths
        for (cov_path, lines) in &self.file_coverage {
            if (cov_path == &norm
                || cov_path.ends_with(&format!("/{norm}"))
                || norm.ends_with(&format!("/{cov_path}")))
                && lines.contains(&line_number)
            {
                return true;
            }
        }
        false
    }
}

/// Per-file diff coverage breakdown.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileDiffCoverage {
    pub path: String,
    pub covered_lines: usize,
    pub total_lines: usize,
    pub coverage_pct: f64,
    pub uncovered_lines: Vec<usize>,
}

/// Aggregated diff coverage evaluation result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiffCoverageResult {
    pub covered_lines: usize,
    pub total_lines: usize,
    pub coverage_pct: f64,
    pub min_threshold: f64,
    pub passed: bool,
    pub files: Vec<FileDiffCoverage>,
    pub message: String,
}

/// Checks if a file path is considered a source code file subject to coverage verification.
pub fn is_source_code_file(path: &str) -> bool {
    let norm = normalize_path(path);
    if is_test_file(&norm) {
        return false;
    }

    let extensions = [
        ".rs", ".py", ".js", ".jsx", ".ts", ".tsx", ".go", ".c", ".cpp", ".cc", ".h", ".hpp",
        ".java", ".kt", ".rb", ".php", ".swift", ".cs",
    ];

    extensions.iter().any(|ext| norm.ends_with(ext))
}

/// Parses unified diff text and extracts added/modified line numbers for source code files.
pub fn parse_diff_added_lines(diff_text: &str) -> Vec<FileDiffLines> {
    let mut results: Vec<FileDiffLines> = Vec::new();
    let mut current_file: Option<String> = None;
    let mut current_added_lines: BTreeSet<usize> = BTreeSet::new();
    let mut current_new_line = 0usize;
    let mut in_hunk = false;

    for line in diff_text.lines() {
        if let Some(rest) = line.strip_prefix("diff --git a/") {
            // Flush previous file
            if let Some(file) = current_file.take() {
                if !current_added_lines.is_empty() {
                    results.push(FileDiffLines {
                        path: file,
                        added_lines: current_added_lines,
                    });
                }
                current_added_lines = BTreeSet::new();
            }
            in_hunk = false;

            if let Some((_, new_path)) = rest.split_once(" b/") {
                let norm = normalize_path(new_path);
                if is_source_code_file(&norm) {
                    current_file = Some(norm);
                } else {
                    current_file = None;
                }
            }
            continue;
        }

        if current_file.is_none() {
            continue;
        }

        // Hunk header: @@ -old_start,old_count +new_start,new_count @@
        if line.starts_with("@@ ") {
            if let Some(plus_idx) = line.find('+') {
                let after_plus = &line[plus_idx + 1..];
                let num_str = after_plus.split([',', ' ']).next().unwrap_or("0");
                if let Ok(start) = num_str.parse::<usize>() {
                    current_new_line = start;
                    in_hunk = true;
                }
            }
            continue;
        }

        if !in_hunk {
            continue;
        }

        if line.starts_with('+') && !line.starts_with("+++") {
            current_added_lines.insert(current_new_line);
            current_new_line += 1;
        } else if line.starts_with('-') && !line.starts_with("---") {
            // Deleted line does not advance line number in new file
        } else if line.starts_with('\\') {
            // "\ No newline at end of file"
        } else {
            // Context line
            current_new_line += 1;
        }
    }

    // Flush last file
    if let Some(file) = current_file {
        if !current_added_lines.is_empty() {
            results.push(FileDiffLines {
                path: file,
                added_lines: current_added_lines,
            });
        }
    }

    results
}

/// Parses an LCOV tracefile (`.info` or `.lcov`).
pub fn parse_lcov(content: &str) -> Result<CoverageReport, VerifierError> {
    let mut report = CoverageReport::new();
    let mut current_source: Option<String> = None;

    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("SF:") {
            current_source = Some(normalize_path(rest));
        } else if let Some(rest) = trimmed.strip_prefix("DA:") {
            if let Some(ref source) = current_source {
                let mut parts = rest.split(',');
                if let (Some(line_str), Some(hits_str)) = (parts.next(), parts.next()) {
                    if let (Ok(line_no), Ok(hits)) = (
                        line_str.trim().parse::<usize>(),
                        hits_str.trim().parse::<u64>(),
                    ) {
                        if hits > 0 {
                            report.add_covered_line(source, line_no);
                        }
                    }
                }
            }
        } else if trimmed == "end_of_record" {
            current_source = None;
        }
    }

    Ok(report)
}

/// Parses a JSON coverage report (e.g. coverage.py format or generic coverage JSON).
pub fn parse_coverage_json(content: &str) -> Result<CoverageReport, VerifierError> {
    let json: serde_json::Value = serde_json::from_str(content)
        .map_err(|e| VerifierError::CoverageError(format!("Failed to parse coverage JSON: {e}")))?;

    let mut report = CoverageReport::new();

    // 1. Python coverage.py format: {"files": {"src/foo.py": {"executed_lines": [1, 2]}}}
    if let Some(files) = json.get("files").and_then(|f| f.as_object()) {
        for (file_path, file_data) in files {
            if let Some(executed) = file_data.get("executed_lines").and_then(|l| l.as_array()) {
                for line_val in executed {
                    if let Some(line_no) = line_val.as_u64() {
                        report.add_covered_line(file_path, line_no as usize);
                    }
                }
            } else if let Some(lines) = file_data.get("covered_lines").and_then(|l| l.as_array()) {
                for line_val in lines {
                    if let Some(line_no) = line_val.as_u64() {
                        report.add_covered_line(file_path, line_no as usize);
                    }
                }
            }
        }
        return Ok(report);
    }

    // 2. LLVM-cov JSON format: {"data": [{"files": [{"filename": "src/foo.rs", "lines": [{"line": 1, "count": 2}]}]}]}
    if let Some(data) = json.get("data").and_then(|d| d.as_array()) {
        for entry in data {
            if let Some(files) = entry.get("files").and_then(|f| f.as_array()) {
                for file_obj in files {
                    if let Some(filename) = file_obj.get("filename").and_then(|fn_| fn_.as_str()) {
                        if let Some(lines) = file_obj.get("lines").and_then(|l| l.as_array()) {
                            for line_info in lines {
                                if let (Some(l), Some(c)) = (
                                    line_info.get("line").and_then(|v| v.as_u64()),
                                    line_info.get("count").and_then(|v| v.as_u64()),
                                ) {
                                    if c > 0 {
                                        report.add_covered_line(filename, l as usize);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        return Ok(report);
    }

    Ok(report)
}

/// Discovers and loads a coverage report from an explicit path or well-known convention files.
pub async fn load_coverage_report(
    workspace: &Path,
    explicit_file: Option<&Path>,
) -> Result<Option<CoverageReport>, VerifierError> {
    if let Some(file_path) = explicit_file {
        let resolved = if file_path.is_absolute() {
            file_path.to_path_buf()
        } else {
            workspace.join(file_path)
        };

        if !resolved.exists() {
            return Err(VerifierError::CoverageError(format!(
                "Specified coverage report file not found: {}",
                resolved.display()
            )));
        }

        let content = std::fs::read_to_string(&resolved)?;
        if resolved
            .extension()
            .map(|ext| ext == "json")
            .unwrap_or(false)
        {
            return parse_coverage_json(&content).map(Some);
        } else {
            return parse_lcov(&content).map(Some);
        }
    }

    // Auto-discovery search paths
    let candidate_paths = [
        "lcov.info",
        "coverage.lcov",
        "coverage/lcov.info",
        "target/coverage/lcov.info",
        "target/llvm-cov/lcov.info",
        "coverage.json",
        ".coverage.json",
    ];

    for candidate in candidate_paths {
        let p = workspace.join(candidate);
        if p.exists() {
            if let Ok(content) = std::fs::read_to_string(&p) {
                if candidate.ends_with(".json") {
                    if let Ok(rep) = parse_coverage_json(&content) {
                        return Ok(Some(rep));
                    }
                } else if let Ok(rep) = parse_lcov(&content) {
                    return Ok(Some(rep));
                }
            }
        }
    }

    Ok(None)
}

/// Evaluates diff coverage given extracted diff lines, loaded coverage report, and required minimum threshold.
pub fn evaluate_diff_coverage(
    diff_files: &[FileDiffLines],
    report: &CoverageReport,
    min_threshold: f64,
) -> DiffCoverageResult {
    let mut total_lines = 0usize;
    let mut covered_lines = 0usize;
    let mut file_reports = Vec::new();

    for file_diff in diff_files {
        let mut file_covered = 0usize;
        let mut uncovered = Vec::new();

        for line_no in &file_diff.added_lines {
            total_lines += 1;
            if report.is_line_covered(&file_diff.path, *line_no) {
                covered_lines += 1;
                file_covered += 1;
            } else {
                uncovered.push(*line_no);
            }
        }

        let file_total = file_diff.added_lines.len();
        let file_pct = if file_total == 0 {
            100.0
        } else {
            (file_covered as f64 / file_total as f64) * 100.0
        };

        file_reports.push(FileDiffCoverage {
            path: file_diff.path.clone(),
            covered_lines: file_covered,
            total_lines: file_total,
            coverage_pct: file_pct,
            uncovered_lines: uncovered,
        });
    }

    let overall_pct = if total_lines == 0 {
        100.0
    } else {
        (covered_lines as f64 / total_lines as f64) * 100.0
    };

    let passed = overall_pct >= min_threshold;
    let message = if total_lines == 0 {
        "No executable diff lines detected in source code files (100.0% coverage)".to_string()
    } else if passed {
        format!(
            "Diff coverage {:.1}% satisfies required threshold {:.1}% ({}/{} diff lines covered)",
            overall_pct, min_threshold, covered_lines, total_lines
        )
    } else {
        format!(
            "Diff coverage {:.1}% is below required threshold {:.1}% ({}/{} diff lines covered)",
            overall_pct, min_threshold, covered_lines, total_lines
        )
    };

    DiffCoverageResult {
        covered_lines,
        total_lines,
        coverage_pct: overall_pct,
        min_threshold,
        passed,
        files: file_reports,
        message,
    }
}

/// Obtains unified git diff text for working tree relative to HEAD or base branch.
pub async fn fetch_git_diff(
    workspace: &Path,
    base_branch: Option<&str>,
) -> Result<String, VerifierError> {
    let mut cmd = Command::new("git");
    cmd.current_dir(workspace);

    if let Some(base) = base_branch {
        cmd.args(["diff", "-U0", base]);
    } else {
        cmd.args(["diff", "-U0", "HEAD"]);
    }

    let out = cmd
        .output()
        .await
        .map_err(|e| VerifierError::GitError(format!("Failed to execute git diff: {e}")))?;

    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        // Fallback to git diff -U0 without HEAD (staged or unstaged against index)
        let mut fallback = Command::new("git");
        fallback.current_dir(workspace).args(["diff", "-U0"]);
        let fb_out = fallback.output().await.map_err(|e| {
            VerifierError::GitError(format!("Failed to execute fallback git diff: {e}"))
        })?;
        Ok(String::from_utf8_lossy(&fb_out.stdout).to_string())
    }
}

/// High-level evaluation of diff coverage for a workspace.
pub async fn check_diff_coverage(
    workspace: &Path,
    min_threshold: Option<f64>,
    explicit_coverage_file: Option<&Path>,
    base_branch: Option<&str>,
) -> Result<Option<DiffCoverageCheck>, VerifierError> {
    let threshold = match min_threshold {
        Some(t) => t,
        None => return Ok(None),
    };

    let diff_text = fetch_git_diff(workspace, base_branch).await?;
    let diff_files = parse_diff_added_lines(&diff_text);

    // If no source lines were added or modified in git diff, pass cleanly
    if diff_files.is_empty() {
        return Ok(Some(DiffCoverageCheck {
            status: StepStatus::Passed,
            coverage_pct: Some(100.0),
            min_threshold: Some(threshold),
            covered_lines: 0,
            total_lines: 0,
            message: Some(
                "No source code line additions in git diff (100.0% coverage)".to_string(),
            ),
        }));
    }

    let coverage_report = match load_coverage_report(workspace, explicit_coverage_file).await? {
        Some(report) => report,
        None => {
            return Ok(Some(DiffCoverageCheck {
                status: StepStatus::Failed,
                coverage_pct: None,
                min_threshold: Some(threshold),
                covered_lines: 0,
                total_lines: diff_files.iter().map(|f| f.added_lines.len()).sum(),
                message: Some(
                    "Coverage report not found. Provide --coverage-file or generate lcov.info / coverage.json prior to verification."
                        .to_string(),
                ),
            }));
        }
    };

    let result = evaluate_diff_coverage(&diff_files, &coverage_report, threshold);

    Ok(Some(DiffCoverageCheck {
        status: if result.passed {
            StepStatus::Passed
        } else {
            StepStatus::Failed
        },
        coverage_pct: Some(result.coverage_pct),
        min_threshold: Some(threshold),
        covered_lines: result.covered_lines,
        total_lines: result.total_lines,
        message: Some(result.message),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_diff_added_lines_basic() {
        let diff = r#"
diff --git a/src/foo.rs b/src/foo.rs
index 1234567..89abcdef 100644
--- a/src/foo.rs
+++ b/src/foo.rs
@@ -10,3 +10,4 @@
 fn bar() {
-    let x = 1;
+    let x = 2;
+    let y = 3;
 }
diff --git a/tests/foo_test.rs b/tests/foo_test.rs
--- a/tests/foo_test.rs
+++ b/tests/foo_test.rs
@@ -1,2 +1,3 @@
+assert!(true);
"#;
        let files = parse_diff_added_lines(diff);
        assert_eq!(
            files.len(),
            1,
            "Test files should be excluded from source code diff lines"
        );
        assert_eq!(files[0].path, "src/foo.rs");
        assert_eq!(files[0].added_lines, BTreeSet::from([11, 12]));
    }

    #[test]
    fn test_parse_lcov_basic() {
        let lcov = r#"
TN:
SF:src/foo.rs
DA:10,5
DA:11,0
DA:12,1
end_of_record
"#;
        let rep = parse_lcov(lcov).unwrap();
        assert!(rep.is_line_covered("src/foo.rs", 10));
        assert!(!rep.is_line_covered("src/foo.rs", 11));
        assert!(rep.is_line_covered("src/foo.rs", 12));
        assert!(!rep.is_line_covered("src/foo.rs", 13));
    }

    #[test]
    fn test_parse_coverage_json_python_style() {
        let json = r#"
{
  "files": {
    "src/calc.py": {
      "executed_lines": [10, 12, 15]
    }
  }
}
"#;
        let rep = parse_coverage_json(json).unwrap();
        assert!(rep.is_line_covered("src/calc.py", 10));
        assert!(!rep.is_line_covered("src/calc.py", 11));
        assert!(rep.is_line_covered("src/calc.py", 12));
    }

    #[test]
    fn test_evaluate_diff_coverage_thresholds() {
        let diff_files = vec![FileDiffLines {
            path: "src/foo.rs".to_string(),
            added_lines: BTreeSet::from([10, 11, 12, 13]),
        }];

        let mut report = CoverageReport::new();
        report.add_covered_line("src/foo.rs", 10);
        report.add_covered_line("src/foo.rs", 11);
        report.add_covered_line("src/foo.rs", 12);
        // line 13 uncovered -> 3/4 = 75.0%

        let res_pass = evaluate_diff_coverage(&diff_files, &report, 70.0);
        assert!(res_pass.passed);
        assert_eq!(res_pass.coverage_pct, 75.0);

        let res_fail = evaluate_diff_coverage(&diff_files, &report, 80.0);
        assert!(!res_fail.passed);
        assert_eq!(res_fail.coverage_pct, 75.0);
    }
}
