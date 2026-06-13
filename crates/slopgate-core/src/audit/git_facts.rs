//! Git fact gathering for `slopgate audit` — extraction only, no scoring.
//! Mirrors `src/audit/git-facts.mjs`.
//!
//! Every function fail-opens to empty data on any git error (not a repo, shallow history):
//! audit sections skip-with-notice on empty inputs, they never crash.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use serde::Serialize;

/// Author commit share within a path prefix and time window.
#[derive(Debug, Clone, PartialEq)]
pub struct AuthorShare {
    pub author: String,
    pub commits: u32,
    pub share: f64,
}

/// Entry-count sample for a committed JSON file at one revision.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct JsonEntryPoint {
    pub ts: String,
    pub count: usize,
}

fn git(repo_root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output();

    match output {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).into_owned(),
        _ => String::new(),
    }
}

/// Commits-touching-file counts within the window.
pub fn churn_by_file(repo_root: &Path, since_days: u32) -> HashMap<String, u32> {
    let since = format!("--since={since_days} days ago");
    let raw = git(repo_root, &["log", &since, "--name-only", "--format="]);
    let mut map = HashMap::new();
    for line in raw.lines() {
        let f = line.trim();
        if f.is_empty() {
            continue;
        }
        *map.entry(f.to_string()).or_insert(0) += 1;
    }
    map
}

/// Per-commit file sets within the window (for co-change mining).
///
/// Commits touching more than `max_files` files are skipped — bulk refactors poison
/// coupling stats.
pub fn commit_file_sets(repo_root: &Path, since_days: u32, max_files: usize) -> Vec<Vec<String>> {
    let since = format!("--since={since_days} days ago");
    let raw = git(repo_root, &["log", &since, "--name-only", "--format=%H"]);
    let mut sets: Vec<Vec<String>> = Vec::new();
    let mut cur_idx: Option<usize> = None;
    for line in raw.lines() {
        let line = line.trim_end();
        if line.len() == 40 && line.chars().all(|c| c.is_ascii_hexdigit()) {
            sets.push(Vec::new());
            cur_idx = Some(sets.len() - 1);
        } else if let Some(idx) = cur_idx {
            let f = line.trim();
            if !f.is_empty() {
                sets[idx].push(f.to_string());
            }
        }
    }
    sets.into_iter()
        .filter(|s| !s.is_empty() && s.len() <= max_files)
        .collect()
}

/// Author commit shares for a path prefix, sorted by commits descending.
pub fn author_shares(repo_root: &Path, since_days: u32, dir: &str) -> Vec<AuthorShare> {
    let since = format!("--since={since_days} days ago");
    let raw = git(repo_root, &["log", &since, "--format=%an", "--", dir]);
    let mut counts: HashMap<String, u32> = HashMap::new();
    let mut total = 0u32;
    for line in raw.lines() {
        let a = line.trim();
        if a.is_empty() {
            continue;
        }
        total += 1;
        *counts.entry(a.to_string()).or_insert(0) += 1;
    }
    if total == 0 {
        return Vec::new();
    }
    let total_f = f64::from(total);
    let mut shares: Vec<AuthorShare> = counts
        .into_iter()
        .map(|(author, commits)| AuthorShare {
            share: f64::from(commits) / total_f,
            author,
            commits,
        })
        .collect();
    shares.sort_by_key(|s| std::cmp::Reverse(s.commits));
    shares
}

/// File content at the last commit before `days_ago`.
///
/// `None` when there is no revision or the file is absent at that revision.
pub fn file_at_days_ago(repo_root: &Path, file: &str, days_ago: u32) -> Option<String> {
    let before = format!("--before={days_ago} days ago");
    let sha = git(repo_root, &["rev-list", "-1", &before, "HEAD"])
        .trim()
        .to_string();
    if sha.is_empty() {
        return None;
    }
    let spec = format!("{sha}:{file}");
    let out = git(repo_root, &["show", &spec]);
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Entry-count history of a committed `{ entries: {...} }` JSON file, oldest → newest.
pub fn json_entry_history(
    repo_root: &Path,
    rel_path: &str,
    entries_key: &str,
) -> Vec<JsonEntryPoint> {
    let raw = git(repo_root, &["log", "--format=%H %cI", "--", rel_path]);
    let mut out = Vec::new();
    for line in raw.trim().lines() {
        let mut parts = line.splitn(2, ' ');
        let Some(sha) = parts.next() else {
            continue;
        };
        let sha = sha.trim();
        if sha.is_empty() {
            continue;
        }
        let ts = parts.next().unwrap_or("").to_string();
        let spec = format!("{sha}:{rel_path}");
        let blob = git(repo_root, &["show", &spec]);
        let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&blob) else {
            continue;
        };
        let count = parsed
            .get(entries_key)
            .and_then(|v| v.as_object())
            .map(|o| o.len())
            .unwrap_or(0);
        out.push(JsonEntryPoint { ts, count });
    }
    out.reverse();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;
    use tempfile::TempDir;

    fn run_git(repo: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(repo)
            .status()
            .expect("spawn git");
        assert!(status.success(), "git {:?} failed in {:?}", args, repo);
    }

    fn init_repo(repo: &Path) {
        run_git(repo, &["init", "-b", "main"]);
        run_git(repo, &["config", "user.email", "test@example.com"]);
        run_git(repo, &["config", "user.name", "Test User"]);
    }

    fn write_commit(repo: &Path, path: &str, content: &str, msg: &str) {
        let file_path = repo.join(path);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&file_path, content).unwrap();
        run_git(repo, &["add", path]);
        run_git(repo, &["commit", "-m", msg]);
    }

    fn sample_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        init_repo(root);
        write_commit(root, "a.txt", "v1\n", "touch a");
        write_commit(root, "a.txt", "v2\n", "touch a again");
        write_commit(root, "b.txt", "hello\n", "add b");
        write_commit(root, "a.txt", "v3\n", "touch a third time");
        dir
    }

    #[test]
    fn churn_by_file_counts_commits_per_file() {
        let dir = sample_repo();
        let churn = churn_by_file(dir.path(), 30);
        assert_eq!(churn.get("a.txt"), Some(&3));
        assert_eq!(churn.get("b.txt"), Some(&1));
    }

    #[test]
    fn commit_file_sets_groups_files_per_commit() {
        let dir = sample_repo();
        let sets = commit_file_sets(dir.path(), 30, 20);
        assert_eq!(sets.len(), 4);
        // `git log` is newest-first; each set is one commit's touched files.
        assert_eq!(sets[0], vec!["a.txt".to_string()]);
        assert_eq!(sets[1], vec!["b.txt".to_string()]);
        assert_eq!(sets[2], vec!["a.txt".to_string()]);
        assert_eq!(sets[3], vec!["a.txt".to_string()]);
    }

    #[test]
    fn commit_file_sets_skips_bulk_commits() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        init_repo(root);
        for i in 0..25 {
            let name = format!("f{i}.txt");
            fs::write(root.join(&name), "x\n").unwrap();
        }
        run_git(root, &["add", "."]);
        run_git(root, &["commit", "-m", "bulk refactor"]);
        let sets = commit_file_sets(root, 30, 20);
        assert!(sets.is_empty());
    }

    #[test]
    fn author_shares_ranks_by_commits() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        init_repo(root);
        run_git(root, &["config", "user.name", "Alice"]);
        write_commit(root, "src/x.rs", "a\n", "alice 1");
        write_commit(root, "src/x.rs", "b\n", "alice 2");
        fs::write(root.join("src/x.rs"), "c\n").unwrap();
        run_git(root, &["add", "src/x.rs"]);
        run_git(
            root,
            &[
                "-c",
                "user.name=Bob",
                "-c",
                "user.email=bob@example.com",
                "commit",
                "-m",
                "bob touch",
            ],
        );
        let shares = author_shares(root, 30, "src");
        assert_eq!(shares.len(), 2);
        assert_eq!(shares[0].author, "Alice");
        assert_eq!(shares[0].commits, 2);
        assert!((shares[0].share - 2.0 / 3.0).abs() < f64::EPSILON);
        assert_eq!(shares[1].author, "Bob");
    }

    #[test]
    fn file_at_days_ago_returns_content() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        init_repo(root);
        write_commit(root, "data.txt", "current\n", "init");
        let past = file_at_days_ago(root, "data.txt", 0);
        assert_eq!(past.as_deref(), Some("current\n"));
        assert_eq!(file_at_days_ago(root, "missing.txt", 0), None);
    }

    #[test]
    fn json_entry_history_oldest_to_newest() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        init_repo(root);
        write_commit(root, "baseline.json", r#"{"entries":{"a":1}}"#, "one entry");
        write_commit(
            root,
            "baseline.json",
            r#"{"entries":{"a":1,"b":2}}"#,
            "two entries",
        );
        let history = json_entry_history(root, "baseline.json", "entries");
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].count, 1);
        assert_eq!(history[1].count, 2);
        assert!(!history[0].ts.is_empty());
    }

    #[test]
    fn non_git_dir_returns_empty_without_panic() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        assert!(churn_by_file(root, 30).is_empty());
        assert!(commit_file_sets(root, 30, 20).is_empty());
        assert!(author_shares(root, 30, "src").is_empty());
        assert_eq!(file_at_days_ago(root, "a.txt", 7), None);
        assert!(json_entry_history(root, "baseline.json", "entries").is_empty());
    }
}
