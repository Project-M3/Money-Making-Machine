//! Workspace storage breakdown and safe cache cleanup (`m3-core::storage`).
//!
//! Computes hardlink-aware byte totals across SEC bulk archives, cached filing texts,
//! DuckDB/Parquet lakehouse files, quotes/computed caches, and protected personal workbench files.

use crate::store::{WorkspacePaths, now_ms};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Logical storage bucket surfaced in Settings (macOS Storage style).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageCategory {
    SecBulkArchives,
    SecFilingsCache,
    DuckDbLakehouse,
    QuotesAndCache,
    PersonalWorkbench,
}

impl StorageCategory {
    /// Returns true if this category holds rebuildable/redownloadable cache data.
    pub const fn is_cleanable(self) -> bool {
        !matches!(self, Self::PersonalWorkbench)
    }
}

/// Measured disk usage and cleanup eligibility for a single [`StorageCategory`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageCategoryReport {
    pub category: StorageCategory,
    pub bytes: u64,
    pub file_count: u64,
    pub cleanable: bool,
    pub reveal_path: PathBuf,
}

/// Complete local storage scan across all categories for a workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageReport {
    pub total_bytes: u64,
    pub cleanable_bytes: u64,
    pub categories: Vec<StorageCategoryReport>,
    pub scanned_at_ms: u64,
}

#[derive(Default)]
struct ScanAcc {
    bytes: u64,
    files: u64,
    seen_inodes: HashSet<(u64, u64)>,
}

impl ScanAcc {
    fn record_file_meta(&mut self, meta: &fs::Metadata) {
        self.files = self.files.saturating_add(1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let key = (meta.dev(), meta.ino());
            if !self.seen_inodes.insert(key) {
                return;
            }
        }
        self.bytes = self.bytes.saturating_add(meta.len());
    }

    fn scan_path(&mut self, path: &Path, skip_name: Option<&str>) {
        let Ok(meta) = fs::symlink_metadata(path) else {
            return;
        };
        if meta.file_type().is_symlink() {
            return;
        }
        if meta.is_file() {
            if let Some(skip) = skip_name
                && path.file_name().and_then(|n| n.to_str()) == Some(skip)
            {
                return;
            }
            self.record_file_meta(&meta);
            return;
        }
        if !meta.is_dir() {
            return;
        }
        let Ok(entries) = fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            let child = entry.path();
            if let Some(skip) = skip_name
                && child.file_name().and_then(|n| n.to_str()) == Some(skip)
            {
                continue;
            }
            self.scan_path(&child, None);
        }
    }
}

fn measure_category(
    paths: &WorkspacePaths,
    updater_dir: Option<&Path>,
    category: StorageCategory,
) -> StorageCategoryReport {
    let mut acc = ScanAcc::default();
    let reveal_path = match category {
        StorageCategory::SecBulkArchives => {
            let dir = paths.lake_raw_sec_dir();
            acc.scan_path(&dir, None);
            dir
        }
        StorageCategory::SecFilingsCache => {
            let dir = paths.lake_dir().join("raw/filings");
            acc.scan_path(&dir, None);
            dir
        }
        StorageCategory::DuckDbLakehouse => {
            let lake_dir = paths.lake_dir();
            acc.scan_path(&paths.lake_parquet_dir(), None);
            if let Ok(entries) = fs::read_dir(&lake_dir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("m3.duckdb"))
                    {
                        acc.scan_path(&p, None);
                    }
                }
            }
            lake_dir
        }
        StorageCategory::QuotesAndCache => {
            acc.scan_path(&paths.data_dir.join("raw"), None);
            acc.scan_path(&paths.data_dir.join("computed"), None);
            acc.scan_path(&paths.sync_status_path(), None);
            if let Some(upd) = updater_dir {
                acc.scan_path(upd, Some("update-check-cache.json"));
            }
            paths.data_dir.clone()
        }
        StorageCategory::PersonalWorkbench => {
            acc.scan_path(&paths.workbench_dir, None);
            paths.workbench_dir.clone()
        }
    };

    StorageCategoryReport {
        category,
        bytes: acc.bytes,
        file_count: acc.files,
        cleanable: category.is_cleanable(),
        reveal_path,
    }
}

/// Scans all local workspace and optional updater storage buckets without following symlinks.
///
/// Hardlinked files shared within a bucket (e.g. active SEC generations) are counted once.
pub fn scan_workspace_storage(paths: &WorkspacePaths, updater_dir: Option<&Path>) -> StorageReport {
    let categories = [
        StorageCategory::SecBulkArchives,
        StorageCategory::SecFilingsCache,
        StorageCategory::DuckDbLakehouse,
        StorageCategory::QuotesAndCache,
        StorageCategory::PersonalWorkbench,
    ]
    .into_iter()
    .map(|cat| measure_category(paths, updater_dir, cat))
    .collect::<Vec<_>>();

    let total_bytes = categories.iter().map(|c| c.bytes).sum();
    let cleanable_bytes = categories
        .iter()
        .filter(|c| c.cleanable)
        .map(|c| c.bytes)
        .sum();

    StorageReport {
        total_bytes,
        cleanable_bytes,
        categories,
        scanned_at_ms: now_ms(),
    }
}

fn remove_path_if_exists(path: &Path) -> Result<()> {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return Ok(());
    };
    if meta.file_type().is_symlink() || meta.is_file() {
        fs::remove_file(path).with_context(|| format!("failed to remove {}", path.display()))?;
    } else if meta.is_dir() {
        fs::remove_dir_all(path)
            .with_context(|| format!("failed to remove directory {}", path.display()))?;
    }
    Ok(())
}

/// Safely clears a rebuildable cache [`StorageCategory`] and returns bytes freed.
///
/// Refuses [`StorageCategory::PersonalWorkbench`] so Circle holdings, dossiers,
/// industry primers, decision journal entries, and budget settings are never deleted.
pub fn clean_storage_category(
    paths: &WorkspacePaths,
    updater_dir: Option<&Path>,
    category: StorageCategory,
) -> Result<u64> {
    if category == StorageCategory::PersonalWorkbench {
        bail!("Personal workbench data is protected and cannot be deleted by cache cleanup");
    }

    let before = measure_category(paths, updater_dir, category).bytes;

    match category {
        StorageCategory::SecBulkArchives => {
            let dir = paths.lake_raw_sec_dir();
            if let Ok(entries) = fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.file_name().and_then(|n| n.to_str()) == Some(".sync.lock") {
                        continue;
                    }
                    remove_path_if_exists(&p)?;
                }
            }
        }
        StorageCategory::SecFilingsCache => {
            remove_path_if_exists(&paths.lake_dir().join("raw/filings"))?;
        }
        StorageCategory::DuckDbLakehouse => {
            let lake_dir = paths.lake_dir();
            if let Ok(entries) = fs::read_dir(&lake_dir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("m3.duckdb"))
                    {
                        remove_path_if_exists(&p)?;
                    }
                }
            }
            remove_path_if_exists(&paths.lake_parquet_dir())?;
        }
        StorageCategory::QuotesAndCache => {
            remove_path_if_exists(&paths.data_dir.join("raw"))?;
            remove_path_if_exists(&paths.data_dir.join("computed"))?;
            remove_path_if_exists(&paths.sync_status_path())?;
            if let Some(upd) = updater_dir
                && let Ok(entries) = fs::read_dir(upd)
            {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.file_name().and_then(|n| n.to_str()) == Some("update-check-cache.json") {
                        continue;
                    }
                    remove_path_if_exists(&p)?;
                }
            }
        }
        StorageCategory::PersonalWorkbench => unreachable!(),
    }

    let after = measure_category(paths, updater_dir, category).bytes;
    Ok(before.saturating_sub(after))
}

/// Formats byte counts cleanly for macOS-style storage summaries (`0 B`, `KB`, `MB`, `GB`).
pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    if bytes == 0 {
        return "0 B".to_owned();
    }
    let b = bytes as f64;
    if b < KB {
        format!("{bytes} B")
    } else if b < 10.0 * KB {
        format!("{:.1} KB", b / KB)
    } else if b < MB {
        format!("{:.0} KB", b / KB)
    } else if b < GB {
        format!("{:.1} MB", b / MB)
    } else {
        format!("{:.2} GB", b / GB)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "m3_storage_test_{tag}_{}_{}",
            std::process::id(),
            now_ms()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn format_bytes_formats_boundaries_cleanly() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(16 * 1024), "16 KB");
        assert_eq!(format_bytes(12 * 1024 * 1024), "12.0 MB");
        assert_eq!(format_bytes(2 * 1024 * 1024 * 1024), "2.00 GB");
    }

    #[test]
    fn scan_dedups_hardlinks_and_clean_protects_workbench() {
        let root = temp_root("hardlink_clean");
        let paths = WorkspacePaths::from_root(root.clone());
        let upd = root.join("updater");
        fs::create_dir_all(paths.lake_raw_sec_generations_dir().join("gen-1")).unwrap();
        fs::create_dir_all(paths.lake_dir().join("raw/filings")).unwrap();
        fs::create_dir_all(paths.lake_parquet_dir()).unwrap();
        fs::create_dir_all(paths.data_dir.join("raw/prices")).unwrap();
        fs::create_dir_all(paths.workbench_dir.clone()).unwrap();
        fs::create_dir_all(&upd).unwrap();

        let zip_top = paths.lake_raw_sec_dir().join("companyfacts.zip");
        let zip_gen = paths
            .lake_raw_sec_generations_dir()
            .join("gen-1/companyfacts.zip");
        fs::write(&zip_top, vec![b'x'; 2000]).unwrap();
        fs::hard_link(&zip_top, &zip_gen).unwrap();
        fs::write(paths.lake_raw_sec_dir().join(".sync.lock"), b"").unwrap();

        fs::write(
            paths.lake_dir().join("raw/filings/0001-10k.txt"),
            vec![b'f'; 500],
        )
        .unwrap();
        fs::write(paths.lake_db_path(), vec![b'd'; 300]).unwrap();
        fs::write(
            paths.lake_parquet_dir().join("part.parquet"),
            vec![b'p'; 100],
        )
        .unwrap();
        fs::write(paths.data_dir.join("raw/prices/AAPL.json"), vec![b'q'; 50]).unwrap();
        fs::write(upd.join("staged.tar.gz"), vec![b'u'; 70]).unwrap();
        fs::write(upd.join("update-check-cache.json"), b"{}").unwrap();

        let watchlist = paths.watchlist_path();
        fs::write(&watchlist, br#"{"companies":[]}"#).unwrap();
        let wl_len = fs::metadata(&watchlist).unwrap().len();

        let rep = scan_workspace_storage(&paths, Some(&upd));
        assert_eq!(rep.categories.len(), 5);
        let bulk = rep
            .categories
            .iter()
            .find(|c| c.category == StorageCategory::SecBulkArchives)
            .unwrap();
        // Hardlink ensures 2000 bytes is counted once even though both paths exist.
        assert_eq!(bulk.bytes, 2000);
        assert_eq!(bulk.file_count, 3);
        assert!(bulk.cleanable);

        let db = rep
            .categories
            .iter()
            .find(|c| c.category == StorageCategory::DuckDbLakehouse)
            .unwrap();
        assert_eq!(db.bytes, 400);

        let quotes = rep
            .categories
            .iter()
            .find(|c| c.category == StorageCategory::QuotesAndCache)
            .unwrap();
        assert_eq!(quotes.bytes, 120);

        let wb = rep
            .categories
            .iter()
            .find(|c| c.category == StorageCategory::PersonalWorkbench)
            .unwrap();
        assert_eq!(wb.bytes, wl_len);
        assert!(!wb.cleanable);

        assert_eq!(rep.total_bytes, 2000 + 500 + 400 + 120 + wl_len);
        assert_eq!(rep.cleanable_bytes, 2000 + 500 + 400 + 120);

        // Refuses cleaning PersonalWorkbench
        assert!(
            clean_storage_category(&paths, Some(&upd), StorageCategory::PersonalWorkbench).is_err()
        );
        assert!(watchlist.exists());

        // Cleaning SecBulkArchives frees 2000B and preserves .sync.lock
        let freed_bulk =
            clean_storage_category(&paths, Some(&upd), StorageCategory::SecBulkArchives).unwrap();
        assert_eq!(freed_bulk, 2000);
        assert!(paths.lake_raw_sec_dir().join(".sync.lock").exists());

        // Cleaning QuotesAndCache removes staged bundle + raw/prices while preserving update-check-cache.json
        let freed_q =
            clean_storage_category(&paths, Some(&upd), StorageCategory::QuotesAndCache).unwrap();
        assert_eq!(freed_q, 120);
        assert!(upd.join("update-check-cache.json").exists());
        assert!(!upd.join("staged.tar.gz").exists());

        // Workbench stayed completely intact
        assert!(watchlist.exists());

        let _ = fs::remove_dir_all(&root);
    }
}
