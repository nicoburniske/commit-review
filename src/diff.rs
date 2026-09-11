//! The changes a commit will contain, as parsed unified diffs.

use crate::git;
use crate::message::Scope;

/// Hash of git's empty tree: what a root commit is measured against.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct FileDiff {
    pub path: String,
    /// Previous path of a renamed file.
    pub old_path: Option<String>,
    pub status: Status,
    pub binary: bool,
    pub hunks: Vec<Hunk>,
    /// immutable base blob for optional source context
    pub source: Option<String>,
}

impl FileDiff {
    /// All lines of all hunks in order: what the review indexes.
    pub fn lines(&self) -> impl Iterator<Item = &Line> {
        self.hunks.iter().flat_map(|h| h.lines.iter())
    }

    pub fn context(&self, source: &str) -> Result<Vec<Vec<Line>>, String> {
        let mut lines = source.lines();
        let mut gaps = Vec::with_capacity(self.hunks.len() + 1);
        let (mut old, mut new) = (1, 1);
        for hunk in &self.hunks {
            let mut gap = Vec::new();
            while old < hunk.old.start {
                let text = lines.next().ok_or("Source does not match the diff")?;
                gap.push(Line {
                    kind: Kind::Context,
                    old: Some(old),
                    new: Some(new),
                    text: text.to_owned(),
                });
                old += 1;
                new += 1;
            }
            for line in hunk.lines.iter().filter(|line| line.old.is_some()) {
                if lines.next() != Some(line.text.as_str()) {
                    return Err("Source does not match the diff".into());
                }
            }
            old = hunk.old.end;
            new = hunk.new.end;
            gaps.push(gap);
        }
        gaps.push(
            lines
                .map(|text| {
                    let line = Line {
                        kind: Kind::Context,
                        old: Some(old),
                        new: Some(new),
                        text: text.to_owned(),
                    };
                    old += 1;
                    new += 1;
                    line
                })
                .collect(),
        );
        Ok(gaps)
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum Status {
    Added,
    Deleted,
    Modified,
    Renamed,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Hunk {
    pub header: String,
    pub old: std::ops::Range<u32>,
    pub new: std::ops::Range<u32>,
    pub lines: Vec<Line>,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Line {
    pub kind: Kind,
    /// Line number in the old file, absent for an added line.
    pub old: Option<u32>,
    /// Line number in the new file, absent for a deleted line.
    pub new: Option<u32>,
    pub text: String,
}

impl Line {
    /// The line as the diff shows it, marker first.
    pub fn quoted(&self) -> String {
        let marker = match self.kind {
            Kind::Context => ' ',
            Kind::Add => '+',
            Kind::Del => '-',
        };
        format!("{marker}{}", self.text)
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum Kind {
    Context,
    Add,
    Del,
}

pub fn changes(scope: Scope, amend: bool) -> Result<Vec<FileDiff>, String> {
    let base = base(amend);
    let mut args = vec![
        "diff",
        "--no-color",
        "--no-ext-diff",
        "--find-renames",
        "-U3",
    ];
    if scope == Scope::Staged {
        args.push("--cached");
    }
    args.push(&base);
    let mut files = parse(&git::diff(&args)?);
    for file in &mut files {
        if !file.binary && !matches!(file.status, Status::Added | Status::Deleted) {
            file.source = Some(format!(
                "{base}:{}",
                file.old_path.as_deref().unwrap_or(&file.path)
            ));
        }
    }
    if scope == Scope::Worktree {
        for path in git::run(&["ls-files", "--others", "--exclude-standard"])?.lines() {
            let text = git::diff(&["diff", "--no-color", "--no-index", "/dev/null", path])?;
            files.append(&mut parse(&text));
        }
    }
    Ok(files)
}

/// The commit the changes are measured against; the empty tree when there
/// is none, as for a root commit.
fn base(amend: bool) -> String {
    let rev = if amend { "HEAD~1" } else { "HEAD" };
    git::run(&["rev-parse", "--verify", "--quiet", rev]).unwrap_or_else(|_| EMPTY_TREE.to_string())
}

pub fn parse(diff: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = Vec::new();
    let (mut old_no, mut new_no) = (0, 0);
    for line in diff.lines() {
        if let Some(header) = line.strip_prefix("diff --git ") {
            files.push(FileDiff {
                path: path_from_header(header),
                old_path: None,
                status: Status::Modified,
                binary: false,
                hunks: Vec::new(),
                source: None,
            });
            continue;
        }
        let Some(file) = files.last_mut() else {
            continue;
        };
        if let Some(rest) = line.strip_prefix("@@ ") {
            let (old, new) = hunk_starts(rest);
            (old_no, new_no) = (old.start, new.start);
            file.hunks.push(Hunk {
                header: line.to_string(),
                old,
                new,
                lines: Vec::new(),
            });
        } else if let Some(hunk) = file.hunks.last_mut() {
            let (kind, old, new) = match line.chars().next() {
                Some('+') => (Kind::Add, None, Some(new_no)),
                Some('-') => (Kind::Del, Some(old_no), None),
                Some(' ') => (Kind::Context, Some(old_no), Some(new_no)),
                // "\ No newline at end of file"
                _ => continue,
            };
            old_no += u32::from(old.is_some());
            new_no += u32::from(new.is_some());
            hunk.lines.push(Line {
                kind,
                old,
                new,
                text: line[1..].to_string(),
            });
        } else if line == "--- /dev/null" {
            file.status = Status::Added;
        } else if line == "+++ /dev/null" {
            file.status = Status::Deleted;
        } else if let Some(path) = line.strip_prefix("+++ b/") {
            file.path = path.to_string();
        } else if let Some(path) = line.strip_prefix("rename from ") {
            file.status = Status::Renamed;
            file.old_path = Some(path.to_string());
        } else if line.starts_with("Binary files ") {
            file.binary = true;
        }
    }
    files
}

/// New path from `a/old b/new`; the `+++` line overrides it when present.
fn path_from_header(header: &str) -> String {
    header
        .split_once(" b/")
        .map(|(_, new)| new)
        .unwrap_or(header)
        .to_string()
}

/// Start line numbers of `-old,count +new,count @@ heading`.
fn hunk_starts(rest: &str) -> (std::ops::Range<u32>, std::ops::Range<u32>) {
    let mut parts = rest.split(' ');
    let mut start = |sign: char| {
        let mut range = parts
            .next()
            .and_then(|p| p.strip_prefix(sign))
            .unwrap_or("")
            .split(',');
        let start = range.next().and_then(|n| n.parse().ok()).unwrap_or(0);
        let count = range.next().map_or(1, |n| n.parse().unwrap_or(0));
        let start = start + u32::from(count == 0);
        start..start + count
    };
    let old = start('-');
    let new = start('+');
    (old, new)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(kind: Kind, old: Option<u32>, new: Option<u32>, text: &str) -> Line {
        Line {
            kind,
            old,
            new,
            text: text.to_string(),
        }
    }

    #[test]
    fn source_context_preserves_gaps_and_line_numbers() {
        let file = parse("diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -3 +3,2 @@\n-3\n+three\n+extra\n@@ -7 +7,0 @@\n-7\n").remove(0);
        let gaps = file.context("1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n\n").unwrap();
        assert_eq!(
            gaps[0],
            vec![
                line(Kind::Context, Some(1), Some(1), "1"),
                line(Kind::Context, Some(2), Some(2), "2")
            ]
        );
        assert_eq!(
            gaps[1],
            vec![
                line(Kind::Context, Some(4), Some(5), "4"),
                line(Kind::Context, Some(5), Some(6), "5"),
                line(Kind::Context, Some(6), Some(7), "6"),
            ]
        );
        assert_eq!(gaps[2][0], line(Kind::Context, Some(8), Some(8), "8"));
        assert_eq!(
            gaps[2].last().unwrap(),
            &line(Kind::Context, Some(11), Some(11), "")
        );
        assert!(file.context("1\n2\nwrong\n").is_err());
    }

    #[test]
    fn source_context_handles_empty_ranges_and_unchanged_renames() {
        let file = parse("diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -0,0 +1 @@\n+prefix\n@@ -2,0 +4 @@\n+suffix\n").remove(0);
        let gaps = file.context("one\ntwo\n").unwrap();
        assert!(gaps[0].is_empty() && gaps[2].is_empty());
        assert_eq!(
            gaps[1],
            vec![
                line(Kind::Context, Some(1), Some(2), "one"),
                line(Kind::Context, Some(2), Some(3), "two"),
            ]
        );
        let file = parse("diff --git a/f b/g\nrename from f\nrename to g\n").remove(0);
        assert!(file.source.is_none());
        assert_eq!(
            file.context("one\n").unwrap(),
            vec![vec![line(Kind::Context, Some(1), Some(1), "one")]]
        );
    }

    #[test]
    fn numbers_lines_of_a_modified_file() {
        let diff = "diff --git a/src/a.rs b/src/a.rs\nindex 1..2 100644\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -3,4 +3,5 @@ fn main() {\n ctx\n-old\n+new\n+more\n ctx2\n\\ No newline at end of file\n";
        let files = parse(diff);
        assert_eq!(files.len(), 1);
        let file = &files[0];
        assert_eq!(file.path, "src/a.rs");
        assert_eq!(file.status, Status::Modified);
        assert_eq!(file.hunks[0].header, "@@ -3,4 +3,5 @@ fn main() {");
        assert_eq!(
            file.hunks[0].lines,
            vec![
                line(Kind::Context, Some(3), Some(3), "ctx"),
                line(Kind::Del, Some(4), None, "old"),
                line(Kind::Add, None, Some(4), "new"),
                line(Kind::Add, None, Some(5), "more"),
                line(Kind::Context, Some(5), Some(6), "ctx2"),
            ]
        );
    }

    #[test]
    fn added_renamed_and_binary_files() {
        let diff = "diff --git a/new.txt b/new.txt\nnew file mode 100644\n--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1,2 @@\n+one\n+two\ndiff --git a/old.rs b/moved.rs\nsimilarity index 90%\nrename from old.rs\nrename to moved.rs\n--- a/old.rs\n+++ b/moved.rs\n@@ -1 +1 @@\n-a\n+b\ndiff --git a/img.png b/img.png\nBinary files a/img.png and b/img.png differ\n";
        let files = parse(diff);
        assert_eq!(files.len(), 3);
        assert_eq!(
            (files[0].status, files[0].path.as_str()),
            (Status::Added, "new.txt")
        );
        assert_eq!(
            files[0].hunks[0].lines[1],
            line(Kind::Add, None, Some(2), "two")
        );
        assert_eq!(files[1].status, Status::Renamed);
        assert_eq!(files[1].old_path.as_deref(), Some("old.rs"));
        assert_eq!(files[1].path, "moved.rs");
        assert!(files[2].binary);
        assert_eq!(files[2].path, "img.png");
        assert!(files[2].hunks.is_empty());
    }

    #[test]
    fn hunk_lines_that_look_like_headers_stay_lines() {
        let diff = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1,2 +1,2 @@\n--- not a header\n+++ not one either\n";
        let lines = &parse(diff)[0].hunks[0].lines;
        assert_eq!(lines[0], line(Kind::Del, Some(1), None, "-- not a header"));
        assert_eq!(
            lines[1],
            line(Kind::Add, None, Some(1), "++ not one either")
        );
    }
}
