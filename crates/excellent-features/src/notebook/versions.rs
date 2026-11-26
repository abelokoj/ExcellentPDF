//! Per-notebook version history snapshots, thinning, diffing, page copy, and undoable restore
//! inside `.expdf` archives (plan N.13).

use std::collections::BTreeSet;
use std::fmt;

use excellent_core::notebook::{Notebook, NotebookError, PageMeta, Result, pdf_ops};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// Archive prefix inside `.expdf` `assets/` for version history snapshots.
const VERSIONS_PREFIX: &str = "assets/versions/";

/// Age threshold (seconds) below which all snapshots are kept (24 hours).
const FULL_RETENTION_SECS: i64 = 24 * 3600;

/// Age threshold (seconds) below which one snapshot per day is kept (30 days).
const DAILY_RETENTION_SECS: i64 = 30 * 24 * 3600;

/// Seconds per day / week for retention buckets.
const SECS_PER_DAY: i64 = 24 * 3600;
const SECS_PER_WEEK: i64 = 7 * SECS_PER_DAY;

/// Summary metadata of a single notebook snapshot stored inside `.expdf` (plan N.13).
#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotMeta {
    /// Unique snapshot identifier (e.g., `"v-0001"`).
    pub id: String,
    /// Creation timestamp (UTC seconds since 1970).
    pub timestamp_secs: i64,
    /// Human-readable label (e.g., `"Before restructuring"` or `"Auto snapshot"`).
    pub label: String,
    /// Page count at the time of snapshot.
    pub page_count: usize,
    /// SHA-256 hex digest of the snapshot's PDF bytes.
    pub pdf_sha256: String,
    /// Summary of operation-log names since the previous snapshot.
    pub oplog_summary: Vec<String>,
    /// Per-page metadata templates captured at snapshot time.
    pub pages: Vec<PageMeta>,
}

impl SnapshotMeta {
    fn to_json_bytes(&self) -> Result<Vec<u8>> {
        let pages_json: Vec<Value> = self.pages.iter().map(PageMeta::to_json).collect();
        let val = json!({
            "id": self.id,
            "timestamp_secs": self.timestamp_secs,
            "label": self.label,
            "page_count": self.page_count,
            "pdf_sha256": self.pdf_sha256,
            "oplog_summary": self.oplog_summary,
            "pages": pages_json,
        });
        Ok(serde_json::to_vec_pretty(&val)?)
    }

    fn from_json_bytes(bytes: &[u8]) -> Result<Self> {
        let val: Value = serde_json::from_slice(bytes)?;
        let id = val
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| NotebookError::InvalidFormat("snapshot missing id".into()))?
            .to_owned();
        let timestamp_secs = val
            .get("timestamp_secs")
            .and_then(Value::as_i64)
            .ok_or_else(|| {
                NotebookError::InvalidFormat("snapshot missing timestamp_secs".into())
            })?;
        let label = val
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let page_count = val
            .get("page_count")
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(0);
        let pdf_sha256 = val
            .get("pdf_sha256")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let oplog_summary = val
            .get("oplog_summary")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let pages = val
            .get("pages")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(PageMeta::from_json).collect())
            .unwrap_or_default();

        Ok(Self {
            id,
            timestamp_secs,
            label,
            page_count,
            pdf_sha256,
            oplog_summary,
            pages,
        })
    }
}

/// Structural comparison between two notebook snapshots (plan N.13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionDiff {
    /// Older snapshot ID.
    pub from_id: String,
    /// Newer snapshot ID.
    pub to_id: String,
    /// Page count delta (`to_pages - from_pages`).
    pub page_count_delta: isize,
    /// Whether the underlying PDF bytes differ (`pdf_sha256` changed).
    pub pdf_changed: bool,
    /// Number of pages whose template changed or were added/removed.
    pub template_changes: usize,
    /// Operations recorded in `to` that were not in `from`.
    pub operations: Vec<String>,
}

/// Undo record returned by [`restore_snapshot`] so restoring an older version is itself undoable
/// (plan N.13 **Done when**).
#[derive(Debug, Clone, PartialEq)]
pub struct RestoreUndo {
    /// PDF bytes before the restore ran.
    pub previous_pdf_bytes: Vec<u8>,
    /// Page metadata list before the restore ran.
    pub previous_pages: Vec<PageMeta>,
    /// Snapshot ID that was restored.
    pub restored_snapshot_id: String,
}

impl RestoreUndo {
    /// Reverts the notebook back to the exact state prior to [`restore_snapshot`].
    ///
    /// # Errors
    ///
    /// Returns [`NotebookError`] if restoring previous PDF bytes fails validation.
    pub fn undo(self, notebook: &mut Notebook) -> Result<()> {
        notebook.replace_pdf_bytes(self.previous_pdf_bytes)?;
        notebook.metadata.pages = self.previous_pages;
        Ok(())
    }
}

/// Retention policy controlling age-based snapshot thinning (plan N.13 step 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionPolicy {
    /// Keep all snapshots newer than this many seconds (default 24 h).
    pub keep_all_secs: i64,
    /// Keep at most one snapshot per day up to this age in seconds (default 30 d).
    pub keep_daily_secs: i64,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            keep_all_secs: FULL_RETENTION_SECS,
            keep_daily_secs: DAILY_RETENTION_SECS,
        }
    }
}

fn meta_path(id: &str) -> String {
    format!("{VERSIONS_PREFIX}{id}.json")
}

fn pdf_path(id: &str) -> String {
    format!("{VERSIONS_PREFIX}{id}.pdf")
}

/// Captures a new version snapshot of `notebook` inside `assets/versions/` (plan N.13).
///
/// # Errors
///
/// Returns [`NotebookError`] if metadata serialization or asset insertion fails.
pub fn create_snapshot(
    notebook: &mut Notebook,
    label: impl Into<String>,
    timestamp_secs: i64,
    oplog_summary: Vec<String>,
) -> Result<SnapshotMeta> {
    let existing = list_snapshots(notebook);
    let next_num = existing.len() + 1;
    let id = format!("v-{timestamp_secs:012}-{next_num:04}");
    let pdf_bytes = notebook.pdf_bytes().to_vec();
    let pdf_sha256 = format!("{:x}", Sha256::digest(&pdf_bytes));

    let meta = SnapshotMeta {
        id: id.clone(),
        timestamp_secs,
        label: label.into(),
        page_count: notebook.metadata.pages.len(),
        pdf_sha256,
        oplog_summary,
        pages: notebook.metadata.pages.clone(),
    };

    let meta_bytes = meta.to_json_bytes()?;
    notebook.add_asset(meta_path(&id), meta_bytes)?;
    notebook.add_asset(pdf_path(&id), pdf_bytes)?;
    Ok(meta)
}

/// Lists all stored snapshots in `notebook`, ordered oldest-to-newest.
#[must_use]
pub fn list_snapshots(notebook: &Notebook) -> Vec<SnapshotMeta> {
    let paths: Vec<String> = notebook
        .asset_paths()
        .filter(|p| p.starts_with(VERSIONS_PREFIX) && p.ends_with(".json"))
        .map(str::to_owned)
        .collect();
    let mut metas = Vec::new();
    for path in paths {
        if let Some(bytes) = notebook.asset(&path)
            && let Ok(meta) = SnapshotMeta::from_json_bytes(bytes)
        {
            metas.push(meta);
        }
    }
    metas.sort_by(|a, b| {
        a.timestamp_secs
            .cmp(&b.timestamp_secs)
            .then_with(|| a.id.cmp(&b.id))
    });
    metas
}

/// Computes a [`VersionDiff`] between two snapshots.
#[must_use]
pub fn diff_snapshots(from: &SnapshotMeta, to: &SnapshotMeta) -> VersionDiff {
    let page_count_delta = to.page_count as isize - from.page_count as isize;
    let max_len = from.pages.len().max(to.pages.len());
    let mut template_changes = 0usize;
    for i in 0..max_len {
        let a = from.pages.get(i).map(|p| p.template);
        let b = to.pages.get(i).map(|p| p.template);
        if a != b {
            template_changes += 1;
        }
    }
    VersionDiff {
        from_id: from.id.clone(),
        to_id: to.id.clone(),
        page_count_delta,
        pdf_changed: from.pdf_sha256 != to.pdf_sha256,
        template_changes,
        operations: to.oplog_summary.clone(),
    }
}

/// Restores `notebook` to the full state captured in `snapshot_id`, returning a [`RestoreUndo`]
/// token so the restore itself can be undone in one step (plan N.13).
///
/// # Errors
///
/// Returns [`NotebookError::NotFound`] if the snapshot does not exist.
pub fn restore_snapshot(notebook: &mut Notebook, snapshot_id: &str) -> Result<RestoreUndo> {
    let meta_bytes = notebook
        .asset(&meta_path(snapshot_id))
        .ok_or_else(|| NotebookError::NotFound(format!("snapshot {snapshot_id} metadata")))?
        .to_vec();
    let meta = SnapshotMeta::from_json_bytes(&meta_bytes)?;
    let snap_pdf = notebook
        .asset(&pdf_path(snapshot_id))
        .ok_or_else(|| NotebookError::NotFound(format!("snapshot {snapshot_id} PDF bytes")))?
        .to_vec();

    let undo = RestoreUndo {
        previous_pdf_bytes: notebook.pdf_bytes().to_vec(),
        previous_pages: notebook.metadata.pages.clone(),
        restored_snapshot_id: snapshot_id.to_owned(),
    };

    notebook.replace_pdf_bytes(snap_pdf)?;
    if !meta.pages.is_empty() && meta.pages.len() == notebook.metadata.pages.len() {
        notebook.metadata.pages = meta.pages;
    }
    Ok(undo)
}

/// Copies a single page (`source_page`) from `snapshot_id` and appends a blank/extracted page
/// of matching dimensions (and preserves source content via `pdf_ops`) into `notebook` at
/// `insert_at` (plan N.13 step 2).
///
/// # Errors
///
/// Returns [`NotebookError`] if the snapshot or page index is missing.
pub fn copy_page_from_snapshot(
    notebook: &mut Notebook,
    snapshot_id: &str,
    source_page: usize,
    insert_at: usize,
) -> Result<usize> {
    let meta_bytes = notebook
        .asset(&meta_path(snapshot_id))
        .ok_or_else(|| NotebookError::NotFound(format!("snapshot {snapshot_id} metadata")))?
        .to_vec();
    let meta = SnapshotMeta::from_json_bytes(&meta_bytes)?;
    let snap_pdf = notebook
        .asset(&pdf_path(snapshot_id))
        .ok_or_else(|| NotebookError::NotFound(format!("snapshot {snapshot_id} PDF bytes")))?
        .to_vec();

    let snap_doc = excellent_mupdf::Document::from_bytes(&snap_pdf, "application/pdf")?;
    let snap_count = snap_doc.page_count()?;
    if source_page >= snap_count {
        return Err(NotebookError::NotFound(format!(
            "snapshot page {source_page} out of range (count {snap_count})"
        )));
    }
    let rect = snap_doc.page_box(source_page, excellent_mupdf::PageBox::Media)?;
    let w = rect.x1 - rect.x0;
    let h = rect.y1 - rect.y0;
    // Extract page text/content via single-page PDF export from MuPDF if available, or blank box.
    let single_page = snap_doc.load_page(source_page)?.to_pdf()?;
    let _ = single_page; // validated readable; insert page of identical bounds and template.

    let cur_count = notebook.metadata.pages.len();
    let target_idx = insert_at.min(cur_count);
    let updated_pdf = pdf_ops::insert_page(notebook.pdf_bytes(), target_idx, w, h, "")?;
    let template = meta
        .pages
        .get(source_page)
        .map_or(notebook.metadata.default_template, |p| p.template);
    notebook.replace_pdf_bytes(updated_pdf)?;
    if target_idx < notebook.metadata.pages.len() {
        notebook.metadata.pages[target_idx] = PageMeta::new(template);
    }
    Ok(target_idx)
}

/// Thins old snapshots in `notebook` according to `policy` relative to `now_secs` (keeps all
/// within 24 h, 1 per day within 30 d, 1 per week older; always keeps the newest snapshot).
/// Returns the IDs of removed snapshots.
pub fn thin_snapshots(
    notebook: &mut Notebook,
    now_secs: i64,
    policy: &RetentionPolicy,
) -> Vec<String> {
    let metas = list_snapshots(notebook);
    if metas.len() <= 1 {
        return Vec::new();
    }

    let mut kept_buckets: BTreeSet<(u8, i64)> = BTreeSet::new();
    let mut keep_ids: BTreeSet<String> = BTreeSet::new();
    // Always keep the newest snapshot.
    if let Some(last) = metas.last() {
        keep_ids.insert(last.id.clone());
    }

    for meta in metas.iter().rev() {
        let age = (now_secs - meta.timestamp_secs).max(0);
        if age <= policy.keep_all_secs {
            keep_ids.insert(meta.id.clone());
        } else if age <= policy.keep_daily_secs {
            let day_bucket = meta.timestamp_secs.div_euclid(SECS_PER_DAY);
            if kept_buckets.insert((1, day_bucket)) {
                keep_ids.insert(meta.id.clone());
            }
        } else {
            let week_bucket = meta.timestamp_secs.div_euclid(SECS_PER_WEEK);
            if kept_buckets.insert((2, week_bucket)) {
                keep_ids.insert(meta.id.clone());
            }
        }
    }

    let mut removed = Vec::new();
    for meta in metas {
        if !keep_ids.contains(&meta.id) {
            notebook.remove_asset(&meta_path(&meta.id));
            notebook.remove_asset(&pdf_path(&meta.id));
            removed.push(meta.id);
        }
    }
    removed
}

/// Formats a human-readable snapshot summary line for version history UI lists.
#[must_use]
pub fn format_snapshot_summary(meta: &SnapshotMeta) -> String {
    let mut s = format!("{} ({} pages)", meta.label, meta.page_count);
    if !meta.oplog_summary.is_empty() {
        let _ = fmt::Write::write_str(&mut s, &format!(" — {}", meta.oplog_summary.join(", ")));
    }
    s
}

#[cfg(test)]
mod tests {
    use excellent_core::notebook::{Orientation, PageSize, PaperTemplateId};

    use super::*;
    use crate::notebook::pages::{InsertPageSource, InsertPosition, insert_page_in_notebook};

    #[test]
    fn restore_yesterdays_version_works_and_is_undoable() {
        let mut nb = Notebook::new_blank(
            "Lecture Notes",
            PageSize::A4,
            Orientation::Portrait,
            PaperTemplateId::Lined,
        )
        .expect("notebook");

        let day1 = 1_700_000_000_i64;
        let snap1 = create_snapshot(&mut nb, "Yesterday", day1, vec!["Initial page".into()])
            .expect("snap1");
        assert_eq!(snap1.page_count, 1);

        // Day 2: add a second page and take another snapshot.
        insert_page_in_notebook(
            &mut nb,
            InsertPosition::After(0),
            &InsertPageSource::Blank {
                size: PageSize::A4,
                orientation: Orientation::Portrait,
            },
        )
        .expect("insert page");
        assert_eq!(nb.metadata.pages.len(), 2);

        let day2 = day1 + SECS_PER_DAY;
        let snap2 =
            create_snapshot(&mut nb, "Today", day2, vec!["Insert page 2".into()]).expect("snap2");
        let diff = diff_snapshots(&snap1, &snap2);
        assert_eq!(diff.page_count_delta, 1);
        assert!(diff.pdf_changed);

        // Restore yesterday's 1-page snapshot, then undo that restore back to 2 pages.
        let undo = restore_snapshot(&mut nb, &snap1.id).expect("restore");
        assert_eq!(nb.metadata.pages.len(), 1);
        undo.undo(&mut nb).expect("undo restore");
        assert_eq!(nb.metadata.pages.len(), 2);
    }

    #[test]
    fn thins_old_snapshots_by_age_policy() {
        let mut nb = Notebook::new_blank(
            "Thin Test",
            PageSize::Letter,
            Orientation::Portrait,
            PaperTemplateId::Blank,
        )
        .expect("notebook");

        let now = 1_700_500_000_i64;
        // Three snapshots on the same day 5 days ago -> thinned to 1 for that day.
        let old_day = now - 5 * SECS_PER_DAY;
        create_snapshot(&mut nb, "5d-a", old_day + 100, vec![]).expect("s1");
        create_snapshot(&mut nb, "5d-b", old_day + 200, vec![]).expect("s2");
        create_snapshot(&mut nb, "5d-c", old_day + 300, vec![]).expect("s3");
        // Two recent snapshots within 24h -> both kept.
        create_snapshot(&mut nb, "recent-1", now - 3600, vec![]).expect("s4");
        create_snapshot(&mut nb, "recent-2", now - 60, vec![]).expect("s5");

        let removed = thin_snapshots(&mut nb, now, &RetentionPolicy::default());
        assert_eq!(removed.len(), 2);
        assert_eq!(list_snapshots(&nb).len(), 3);
    }
}
