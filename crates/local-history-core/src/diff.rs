use std::fs;
use std::path::Path;

use thiserror::Error;

use crate::model::SnapshotRecord;

const MAX_EXACT_DIFF_CELLS: usize = 1_000_000;

#[derive(Debug, Error)]
pub enum SnapshotDiffError {
    #[error("failed to read current file {path}: {source}")]
    ReadCurrentFile {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("snapshot {snapshot_id} is binary; textual diff is not available")]
    BinarySnapshot { snapshot_id: String },

    #[error("current file {path} is binary; textual diff is not available")]
    BinaryCurrentFile { path: std::path::PathBuf },
}

pub fn snapshot_to_current_unified_diff(
    snapshot: &SnapshotRecord,
    snapshot_contents: &[u8],
    live_path: &Path,
) -> Result<String, SnapshotDiffError> {
    let current_exists = live_path.exists();
    let current_contents = if current_exists {
        fs::read(live_path).map_err(|source| SnapshotDiffError::ReadCurrentFile {
            path: live_path.to_path_buf(),
            source,
        })?
    } else {
        Vec::new()
    };
    let snapshot_text = if snapshot.captures_missing_file {
        ""
    } else {
        std::str::from_utf8(snapshot_contents).map_err(|_| SnapshotDiffError::BinarySnapshot {
            snapshot_id: snapshot.id.as_str().to_string(),
        })?
    };
    let current_text = if current_exists {
        std::str::from_utf8(&current_contents).map_err(|_| {
            SnapshotDiffError::BinaryCurrentFile {
                path: live_path.to_path_buf(),
            }
        })?
    } else {
        ""
    };
    // Missing/present transitions count as changes even when both sides decode to empty text.
    let state_changed = snapshot.captures_missing_file == current_exists;

    if snapshot_text == current_text && !state_changed {
        return Ok("no changes\n".to_string());
    }

    let mut output = String::new();

    if state_changed {
        output.push_str(&format!(
            "file state changed: snapshot={} current={}\n",
            if snapshot.captures_missing_file {
                "missing"
            } else {
                "present"
            },
            if current_exists { "present" } else { "missing" }
        ));
    }

    let old_label = format!(
        "snapshot:{}:{}{}",
        snapshot.id,
        snapshot.relative_path.display(),
        if snapshot.captures_missing_file {
            " [missing]"
        } else {
            ""
        }
    );
    let new_label = format!(
        "current:{}{}",
        live_path.display(),
        if current_exists { "" } else { " [missing]" }
    );

    output.push_str(&render_unified_diff(
        &old_label,
        &new_label,
        snapshot_text,
        current_text,
    ));

    Ok(output)
}

pub(crate) fn render_unified_diff(
    old_label: &str,
    new_label: &str,
    old_text: &str,
    new_text: &str,
) -> String {
    let old_lines = split_diff_lines(old_text);
    let new_lines = split_diff_lines(new_text);
    let (prefix_len, suffix_len) = common_edges(&old_lines, &new_lines);
    let old_middle_end = old_lines.len() - suffix_len;
    let new_middle_end = new_lines.len() - suffix_len;
    let old_middle = &old_lines[prefix_len..old_middle_end];
    let new_middle = &new_lines[prefix_len..new_middle_end];

    let mut diff = String::new();
    diff.push_str(&format!("--- {old_label}\n"));
    diff.push_str(&format!("+++ {new_label}\n"));
    diff.push_str(&format!(
        "@@ -{} +{} @@\n",
        unified_range(old_lines.len()),
        unified_range(new_lines.len())
    ));

    for line in &old_lines[..prefix_len] {
        push_context_line(&mut diff, line);
    }

    if exact_diff_is_bounded(old_middle.len(), new_middle.len()) {
        push_exact_diff_body(&mut diff, old_middle, new_middle);
    } else {
        push_replace_diff_body(&mut diff, old_middle, new_middle);
    }

    for line in &old_lines[old_middle_end..] {
        push_context_line(&mut diff, line);
    }

    diff
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DiffLine<'a> {
    text: &'a str,
    has_newline: bool,
}

fn split_diff_lines(text: &str) -> Vec<DiffLine<'_>> {
    let mut lines = Vec::new();
    let mut line_start = 0usize;

    for (index, character) in text.char_indices() {
        if character == '\n' {
            lines.push(DiffLine {
                text: &text[line_start..index],
                has_newline: true,
            });
            line_start = index + character.len_utf8();
        }
    }

    if line_start < text.len() {
        lines.push(DiffLine {
            text: &text[line_start..],
            has_newline: false,
        });
    }

    lines
}

fn common_edges(old_lines: &[DiffLine<'_>], new_lines: &[DiffLine<'_>]) -> (usize, usize) {
    let mut prefix_len = 0usize;

    while prefix_len < old_lines.len()
        && prefix_len < new_lines.len()
        && old_lines[prefix_len] == new_lines[prefix_len]
    {
        prefix_len += 1;
    }

    let mut suffix_len = 0usize;

    while suffix_len < old_lines.len().saturating_sub(prefix_len)
        && suffix_len < new_lines.len().saturating_sub(prefix_len)
        && old_lines[old_lines.len() - suffix_len - 1]
            == new_lines[new_lines.len() - suffix_len - 1]
    {
        suffix_len += 1;
    }

    (prefix_len, suffix_len)
}

fn exact_diff_is_bounded(old_len: usize, new_len: usize) -> bool {
    old_len
        .checked_add(1)
        .and_then(|old_cells| {
            new_len
                .checked_add(1)
                .and_then(|new_cells| old_cells.checked_mul(new_cells))
        })
        .is_some_and(|cells| cells <= MAX_EXACT_DIFF_CELLS)
}

fn push_exact_diff_body(diff: &mut String, old_lines: &[DiffLine<'_>], new_lines: &[DiffLine<'_>]) {
    let columns = new_lines.len() + 1;
    let mut lcs = vec![0usize; (old_lines.len() + 1) * columns];

    for old_index in (0..old_lines.len()).rev() {
        for new_index in (0..new_lines.len()).rev() {
            let index = lcs_index(columns, old_index, new_index);
            lcs[index] = if old_lines[old_index] == new_lines[new_index] {
                lcs[lcs_index(columns, old_index + 1, new_index + 1)] + 1
            } else {
                std::cmp::max(
                    lcs[lcs_index(columns, old_index + 1, new_index)],
                    lcs[lcs_index(columns, old_index, new_index + 1)],
                )
            };
        }
    }

    let mut old_index = 0usize;
    let mut new_index = 0usize;

    while old_index < old_lines.len() && new_index < new_lines.len() {
        if old_lines[old_index] == new_lines[new_index] {
            push_context_line(diff, &old_lines[old_index]);
            old_index += 1;
            new_index += 1;
        } else if lcs[lcs_index(columns, old_index + 1, new_index)]
            >= lcs[lcs_index(columns, old_index, new_index + 1)]
        {
            push_removed_line(diff, &old_lines[old_index]);
            old_index += 1;
        } else {
            push_added_line(diff, &new_lines[new_index]);
            new_index += 1;
        }
    }

    while old_index < old_lines.len() {
        push_removed_line(diff, &old_lines[old_index]);
        old_index += 1;
    }

    while new_index < new_lines.len() {
        push_added_line(diff, &new_lines[new_index]);
        new_index += 1;
    }
}

fn push_replace_diff_body(
    diff: &mut String,
    old_lines: &[DiffLine<'_>],
    new_lines: &[DiffLine<'_>],
) {
    for line in old_lines {
        push_removed_line(diff, line);
    }

    for line in new_lines {
        push_added_line(diff, line);
    }
}

fn lcs_index(columns: usize, old_index: usize, new_index: usize) -> usize {
    old_index * columns + new_index
}

fn push_context_line(diff: &mut String, line: &DiffLine<'_>) {
    push_line(diff, ' ', line);
}

fn push_removed_line(diff: &mut String, line: &DiffLine<'_>) {
    push_line(diff, '-', line);

    if !line.has_newline {
        diff.push_str("\\ No newline at end of snapshot\n");
    }
}

fn push_added_line(diff: &mut String, line: &DiffLine<'_>) {
    push_line(diff, '+', line);

    if !line.has_newline {
        diff.push_str("\\ No newline at end of current file\n");
    }
}

fn push_line(diff: &mut String, prefix: char, line: &DiffLine<'_>) {
    diff.push(prefix);
    diff.push_str(line.text);
    diff.push('\n');
}

fn unified_range(line_count: usize) -> String {
    if line_count == 0 {
        "0,0".to_string()
    } else {
        format!("1,{line_count}")
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::{
        ContentHash, LocalHistoryStore, ProjectId, SnapshotId, SnapshotKind, SnapshotRecord,
        SnapshotWriteRequest,
    };

    use super::{render_unified_diff, snapshot_to_current_unified_diff};

    #[test]
    fn renders_unified_diff_for_text_changes() {
        let diff = render_unified_diff(
            "snapshot:abc:src/lib.rs",
            "current:/project/src/lib.rs",
            "fn main() {\n    println!(\"old\");\n}\n",
            "fn main() {\n    println!(\"new\");\n}\n",
        );

        assert!(diff.contains("--- snapshot:abc:src/lib.rs"));
        assert!(diff.contains("+++ current:/project/src/lib.rs"));
        assert!(diff.contains("-    println!(\"old\");"));
        assert!(diff.contains("+    println!(\"new\");"));
    }

    #[test]
    fn renders_final_newline_only_changes() {
        let diff = render_unified_diff(
            "snapshot:abc:note.txt",
            "current:/project/note.txt",
            "v1\n",
            "v1",
        );

        assert!(diff.contains("-v1"));
        assert!(diff.contains("+v1"));
        assert!(diff.contains("\\ No newline at end of current file"));
    }

    #[test]
    fn renders_large_different_inputs_without_quadratic_lcs_matrix() {
        let old_text = numbered_lines("old");
        let new_text = numbered_lines("new");

        let diff = render_unified_diff(
            "snapshot:abc:large.txt",
            "current:/project/large.txt",
            &old_text,
            &new_text,
        );

        assert!(diff.contains("-old-0"));
        assert!(diff.contains("+new-1499"));
    }

    fn numbered_lines(prefix: &str) -> String {
        use std::fmt::Write as _;

        let mut text = String::new();

        for index in 0..1_500 {
            writeln!(&mut text, "{prefix}-{index}").expect("writing to string must succeed");
        }

        text
    }

    #[test]
    fn diff_output_reports_no_changes_for_identical_text() {
        let snapshot = SnapshotRecord {
            id: SnapshotId::new("snapshot-1"),
            project_id: ProjectId::new("project-1"),
            relative_path: PathBuf::from("note.txt"),
            blob_hash: ContentHash::new("hash"),
            size_bytes: 2,
            timestamp: "2026-05-02T14:18:51Z".to_string(),
            kind: SnapshotKind::Raw,
            captures_missing_file: false,
        };
        let (base_dir, project_root) = create_test_roots("diff-no-changes");
        let live_path = project_root.join("note.txt");
        fs::write(&live_path, "v1\n").expect("live file must be written");

        let output = snapshot_to_current_unified_diff(&snapshot, b"v1\n", &live_path)
            .expect("diff output must succeed");

        assert_eq!(output, "no changes\n");
        cleanup_test_roots(&base_dir);
    }

    #[test]
    fn diff_output_reports_unified_diff_against_live_file() {
        let (base_dir, project_root) = create_test_roots("diff-live-file");
        let store = LocalHistoryStore::open(&base_dir, &project_root).expect("store must open");
        let snapshot = store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: PathBuf::from("note.txt"),
                contents: b"v1\n".to_vec(),
                timestamp: "2026-05-02T14:18:51Z".to_string(),
                kind: SnapshotKind::Raw,
                is_binary: false,
                captures_missing_file: false,
            })
            .expect("snapshot must be stored");
        let live_path = project_root.join("note.txt");
        fs::write(&live_path, "v2\n").expect("live file must be written");

        let snapshot_contents = store
            .read_snapshot_content(&snapshot.id)
            .expect("snapshot contents must be readable");
        let output = snapshot_to_current_unified_diff(&snapshot, &snapshot_contents, &live_path)
            .expect("diff output must succeed");

        assert!(output.contains(&format!("--- snapshot:{}:note.txt", snapshot.id)));
        assert!(output.contains(&format!("+++ current:{}", live_path.display())));
        assert!(output.contains("-v1"));
        assert!(output.contains("+v2"));
        cleanup_test_roots(&base_dir);
    }

    #[test]
    fn diff_output_reports_binary_snapshot_error() {
        let snapshot = SnapshotRecord {
            id: SnapshotId::new("snapshot-binary"),
            project_id: ProjectId::new("project-1"),
            relative_path: PathBuf::from("image.png"),
            blob_hash: ContentHash::new("hash"),
            size_bytes: 4,
            timestamp: "2026-05-02T14:18:51Z".to_string(),
            kind: SnapshotKind::Raw,
            captures_missing_file: false,
        };
        let (base_dir, project_root) = create_test_roots("diff-binary");
        let live_path = project_root.join("image.png");
        fs::write(&live_path, b"\x89PNG").expect("live file must be written");

        let error = snapshot_to_current_unified_diff(&snapshot, b"\x89PNG", &live_path)
            .expect_err("binary snapshot diff must fail");

        assert!(error.to_string().contains("binary"));
        assert!(error.to_string().contains("snapshot-binary"));
        cleanup_test_roots(&base_dir);
    }

    #[test]
    fn diff_output_reports_file_state_change_for_deleted_live_file() {
        let snapshot = SnapshotRecord {
            id: SnapshotId::new("snapshot-deleted"),
            project_id: ProjectId::new("project-1"),
            relative_path: PathBuf::from("note.txt"),
            blob_hash: ContentHash::new("hash"),
            size_bytes: 2,
            timestamp: "2026-05-02T14:18:51Z".to_string(),
            kind: SnapshotKind::Raw,
            captures_missing_file: false,
        };
        let (base_dir, project_root) = create_test_roots("diff-deleted");
        let live_path = project_root.join("note.txt");

        let output = snapshot_to_current_unified_diff(&snapshot, b"v1\n", &live_path)
            .expect("diff output must succeed");

        assert!(output.contains("file state changed: snapshot=present current=missing"));
        assert!(output.contains("-v1"));
        cleanup_test_roots(&base_dir);
    }

    fn create_test_roots(label: &str) -> (PathBuf, PathBuf) {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock must be valid")
            .as_nanos();
        let base_dir = std::env::temp_dir().join(format!("local-history-core-{label}-{unique}"));
        let project_root = base_dir.join("project");
        fs::create_dir_all(&project_root).expect("project root must exist");
        (base_dir, project_root)
    }

    fn cleanup_test_roots(base_dir: &Path) {
        let _ = fs::remove_dir_all(base_dir);
    }
}
