//! Recent activity feed — git page history projected to `ActivityEvent[]`.
//!
//! Phase 1 (2026-07-25 Console expansion): git page events only. A future
//! phase may merge in client-activity reads (MutationCount) — that requires
//! `/ops/clients` to be reachable from the engine, which it isn't today.
//!
//! Implementation mirrors [`crate::git::page_history`]'s shell pattern: ONE
//! `git log --since=... --numstat --pretty=format:...` at the repo root
//! (NOT per-slug — per-slug fanout was flagged as a perf risk on large
//! wikis). Parses NUL-separated commit rows + tab-separated numstat rows.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::engine::EngineState;

/// One event in the activity feed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityEvent {
    /// What kind of page change this represents (create / edit / delete).
    pub kind: ActivityKind,
    /// ISO-8601 timestamp (git's `%aI` author-date format).
    pub timestamp: String,
    /// Git author name.
    pub actor: String,
    /// Slug path (relative to wiki root) of the page touched, e.g.
    /// `concepts/moe.md`.
    pub target: String,
    /// Per-file diff stats and commit subject.
    pub detail: ActivityDetail,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityKind {
    PageCreated,
    PageEdited,
    PageDeleted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityDetail {
    /// Lines added (numstat col 1). 0 for deletions.
    pub added: u32,
    /// Lines removed (numstat col 2). 0 for creations.
    pub removed: u32,
    /// Commit subject line (first line of the commit message).
    pub subject: String,
}

/// Fetch recent page changes for the configured default wiki.
///
/// Returns events sorted newest-first, truncated to at most `limit` events.
/// Internally `limit` is also passed to `git log -n` to bound the number of
/// commits walked (a single commit can produce multiple events if it
/// touches multiple files). `since` filters commits older than the given
/// duration; pass `Duration::MAX` (or a very large value) to disable the
/// age filter.
///
/// Phase-1 scope: git page events only. No client-activity merge.
pub fn recent_activity(
    engine: &EngineState,
    since: Duration,
    limit: usize,
) -> Result<Vec<ActivityEvent>> {
    let wiki_name = engine.default_wiki_name();
    let space = engine.space(wiki_name)?;
    let mut events = recent_page_activity(&space.repo_root, &space.wiki_root, since, limit)?;
    // `git log -n` caps commits, but one commit may produce multiple events
    // (one per touched file). Truncate the final vec to honor `limit` as a
    // user-facing event cap — the Activity UI wants "top N events".
    events.truncate(limit);
    Ok(events)
}

/// Run `git log --since=... --numstat --pretty=format:...` at the repo
/// root, filter to wiki `.md` files, and project to `ActivityEvent[]`.
///
/// Split out from [`recent_activity`] so the parser (and its unit tests)
/// can call it without an engine.
fn recent_page_activity(
    repo_root: &Path,
    wiki_root: &Path,
    since: Duration,
    limit: usize,
) -> Result<Vec<ActivityEvent>> {
    // The wiki root is a subdir of the repo root. Compute the relative prefix
    // so we can filter numstat paths (git emits repo-relative paths).
    let wiki_prefix = wiki_root.strip_prefix(repo_root).unwrap_or(wiki_root);

    let since_arg = format!("--since={}seconds", since.as_secs());
    let limit_arg = format!("-n{}", limit);
    let mut cmd = std::process::Command::new("git");
    cmd.current_dir(repo_root)
        .args([
            "log",
            &since_arg,
            &limit_arg,
            // NUL-separated commit row: hash, ISO date, author, subject.
            // NOTE: field order intentionally differs from `page_history`
            // (author before subject) — see flush_commit.
            "--pretty=format:%H%x00%aI%x00%an%x00%s",
            // Per-file add/delete/path rows after each commit row.
            "--numstat",
        ]);

    let output = cmd
        .output()
        .context("failed to run git log — is git installed?")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Empty history is not an error (new file / branch with no commits).
        if stderr.is_empty() {
            return Ok(Vec::new());
        }
        anyhow::bail!("git log failed: {stderr}");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_log_output(&stdout, wiki_prefix)
}

/// Parse the combined pretty+numstat output into events.
///
/// Output shape (per commit, as emitted by `--pretty=format:... --numstat`):
///
/// ```text
/// <hash>\0<date>\0<author>\0<subject>
/// <added>\t<removed>\t<path>
/// <added>\t<removed>\t<path>
/// <blank line>
/// <next commit row>
/// ```
///
/// The state machine tracks the most-recently-seen commit row and buffers
/// the numstat rows that follow it. A commit row is detected by exactly 3
/// NUL bytes (4 fields). At the start of the next commit row (or end of
/// input) the buffered rows are flushed to events.
fn parse_log_output(stdout: &str, wiki_prefix: &Path) -> Result<Vec<ActivityEvent>> {
    let mut events = Vec::new();
    // (hash, date, author, subject) — hash kept for debug symmetry with
    // HistoryEntry even though it isn't surfaced in ActivityEvent today.
    let mut current: Option<(String, String, String, String)> = None;
    // Numstat rows collected for the current commit.
    let mut current_files: Vec<(u32, u32, String)> = Vec::new();

    fn flush_commit(
        current: &mut Option<(String, String, String, String)>,
        files: &mut Vec<(u32, u32, String)>,
        events: &mut Vec<ActivityEvent>,
        wiki_prefix: &Path,
    ) {
        let Some((_hash, date, author, subject)) = current.take() else {
            return;
        };
        for (added, removed, path_str) in files.drain(..) {
            let path = Path::new(&path_str);
            // Only count wiki markdown files. Git paths are repo-relative;
            // wiki_prefix is also repo-relative, so this prefix check works.
            if !path.starts_with(wiki_prefix) {
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let kind = if added > 0 && removed == 0 {
                ActivityKind::PageCreated
            } else if added == 0 && removed > 0 {
                ActivityKind::PageDeleted
            } else {
                ActivityKind::PageEdited
            };
            events.push(ActivityEvent {
                kind,
                timestamp: date.clone(),
                actor: author.clone(),
                target: path_string_relative_to_wiki(&path_str, wiki_prefix),
                detail: ActivityDetail {
                    added,
                    removed,
                    subject: subject.clone(),
                },
            });
        }
    }

    for line in stdout.lines() {
        // A commit row contains exactly 3 NUL bytes (4 fields). The author
        // name / subject can theoretically contain tabs but never NUL
        // (git's %x00 separator would escape it).
        let nul_count = line.bytes().filter(|&b| b == 0).count();
        if nul_count == 3 {
            // New commit row — flush the previous one first.
            flush_commit(&mut current, &mut current_files, &mut events, wiki_prefix);
            let parts: Vec<&str> = line.splitn(4, '\0').collect();
            if parts.len() == 4 {
                current = Some((
                    parts[0].to_string(),
                    parts[1].to_string(),
                    parts[2].to_string(),
                    parts[3].to_string(),
                ));
            }
        } else if line.is_empty() {
            // Blank separator between commits — the next commit row (or
            // end-of-input) triggers the flush. Nothing to do here.
        } else if current.is_some() {
            // Numstat row: "<added>\t<removed>\t<path>". Binary files
            // produce "-" for added/removed — skip those.
            let cols: Vec<&str> = line.splitn(3, '\t').collect();
            if cols.len() == 3 {
                // Skip binary file entries ("-\t-\tpath") — they represent
                // unknown line counts.
                if cols[0] == "-" || cols[1] == "-" {
                    continue;
                }
                let added: u32 = cols[0].parse().unwrap_or(0);
                let removed: u32 = cols[1].parse().unwrap_or(0);
                current_files.push((added, removed, cols[2].to_string()));
            }
        }
    }
    // Flush the last commit (some git versions omit the trailing blank line).
    flush_commit(&mut current, &mut current_files, &mut events, wiki_prefix);

    // Sort newest-first. ISO-8601 (`%aI`) timestamps share a fixed offset
    // within a repo, so lexical comparison is correct.
    events.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    Ok(events)
}

/// Strip `wiki_prefix` from a repo-relative path to get a wiki-relative
/// slug path. E.g. `"test/wiki/concepts/moe.md"` with prefix `"test/wiki"`
/// becomes `"concepts/moe.md"`. Handles both `/` and `\` path separators
/// (Windows git emits `/`).
fn path_string_relative_to_wiki(repo_relative: &str, wiki_prefix: &Path) -> String {
    let wiki_prefix_str = wiki_prefix.to_string_lossy();
    if let Some(stripped) = repo_relative.strip_prefix(&*wiki_prefix_str) {
        stripped.trim_start_matches(['/', '\\']).to_string()
    } else {
        repo_relative.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty() {
        let events = parse_log_output("", Path::new("wiki")).unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn parse_single_commit_single_file() {
        // 1 commit row + 1 numstat row.
        let input = "abc123\02026-07-25T10:00:00+00:00\0Alice\0Add page\n5\t2\twiki/concepts/foo.md\n";
        let events = parse_log_output(input, Path::new("wiki")).unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].kind, ActivityKind::PageEdited));
        assert_eq!(events[0].target, "concepts/foo.md");
        assert_eq!(events[0].detail.added, 5);
        assert_eq!(events[0].detail.removed, 2);
        assert_eq!(events[0].detail.subject, "Add page");
        assert_eq!(events[0].actor, "Alice");
    }

    #[test]
    fn parse_created_vs_deleted() {
        // created: added > 0, removed = 0
        let created = "h\0d\0a\0s\n10\t0\twiki/foo.md\n";
        let ev = parse_log_output(created, Path::new("wiki")).unwrap();
        assert!(matches!(ev[0].kind, ActivityKind::PageCreated));

        // deleted: added = 0, removed > 0
        let deleted = "h\0d\0a\0s\n0\t10\twiki/foo.md\n";
        let ev = parse_log_output(deleted, Path::new("wiki")).unwrap();
        assert!(matches!(ev[0].kind, ActivityKind::PageDeleted));
    }

    #[test]
    fn parse_multi_file_commit() {
        // One commit touching two wiki files — should produce 2 events,
        // both with the same author/subject but different targets.
        let input = "hash\02026-07-25T10:00:00+00:00\0Bob\0multi edit\n\
            3\t1\twiki/concepts/a.md\n\
            7\t2\twiki/notes/b.md\n";
        let ev = parse_log_output(input, Path::new("wiki")).unwrap();
        assert_eq!(ev.len(), 2, "one event per wiki file in the commit");
        assert!(ev.iter().all(|e| e.actor == "Bob"));
        assert!(ev.iter().all(|e| e.detail.subject == "multi edit"));
        let targets: Vec<&str> = ev.iter().map(|e| e.target.as_str()).collect();
        assert!(targets.contains(&"concepts/a.md"));
        assert!(targets.contains(&"notes/b.md"));
    }

    #[test]
    fn parse_multiple_commits_newest_first() {
        // Two commits, older-first in the input (git log default is newest
        // first, but the sort makes the test order-independent).
        let input = "old\02026-07-24T10:00:00+00:00\0A\0old commit\n\
            1\t0\twiki/x.md\n\
            new\02026-07-25T10:00:00+00:00\0A\0new commit\n\
            1\t0\twiki/x.md\n";
        let ev = parse_log_output(input, Path::new("wiki")).unwrap();
        assert_eq!(ev.len(), 2);
        // Sort is newest-first by timestamp.
        assert_eq!(ev[0].detail.subject, "new commit");
        assert_eq!(ev[1].detail.subject, "old commit");
    }

    #[test]
    fn parse_skips_non_md_and_non_wiki() {
        let input = "h\0d\0a\0s\n1\t1\twiki/page.md\n1\t1\tREADME.md\n1\t1\twiki/image.png\n";
        let ev = parse_log_output(input, Path::new("wiki")).unwrap();
        assert_eq!(ev.len(), 1, "only wiki/page.md counts");
        assert_eq!(ev[0].target, "page.md");
    }

    #[test]
    fn parse_skips_binary_files() {
        let input = "h\0d\0a\0s\n-\t-\twiki/binary.md\n";
        let ev = parse_log_output(input, Path::new("wiki")).unwrap();
        assert!(ev.is_empty(), "binary file (dash numstat) skipped");
    }

    #[test]
    fn parse_subject_with_spaces() {
        // Subject containing spaces — must not break the NUL split.
        let input = "h\0d\0a\0this is a long subject line\n5\t2\twiki/p.md\n";
        let ev = parse_log_output(input, Path::new("wiki")).unwrap();
        assert_eq!(ev[0].detail.subject, "this is a long subject line");
    }

    #[test]
    fn path_string_helper_strips_prefix() {
        assert_eq!(
            path_string_relative_to_wiki("test/wiki/concepts/moe.md", Path::new("test/wiki")),
            "concepts/moe.md"
        );
        // No prefix match — return as-is.
        assert_eq!(
            path_string_relative_to_wiki("other/foo.md", Path::new("test/wiki")),
            "other/foo.md"
        );
        // Windows-style separator after prefix is trimmed.
        assert_eq!(
            path_string_relative_to_wiki("wiki\\concepts\\x.md", Path::new("wiki")),
            "concepts\\x.md"
        );
    }
}
