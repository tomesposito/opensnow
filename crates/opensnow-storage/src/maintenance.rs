//! Scheduler-facing, single-table maintenance. No physical files are deleted.
use std::{collections::HashMap, time::Duration};

use futures::TryStreamExt;
use iceberg::{scan::FileScanTask, spec::DataFile, TableIdent};

use crate::{writer, Result, StorageCatalog};

/// Compaction and metadata retention controls for one maintenance pass.
#[derive(Clone, Debug)]
pub struct MaintenancePolicy {
    pub target_file_size_bytes: usize,
    /// Only files smaller than both this threshold and the target qualify.
    pub minimum_file_size_bytes: u64,
    pub retain_last_snapshots: usize,
    /// Reserved expiry cutoff in Unix milliseconds. Expiry is not shipped.
    pub expire_before_ms: Option<i64>,
    /// Reserved for future safe physical cleanup; no files are deleted today.
    pub orphan_grace_period: Duration,
}

impl Default for MaintenancePolicy {
    fn default() -> Self {
        Self {
            target_file_size_bytes: 256 * 1024 * 1024,
            minimum_file_size_bytes: 128 * 1024 * 1024,
            retain_last_snapshots: 5,
            expire_before_ms: None,
            orphan_grace_period: Duration::from_secs(24 * 60 * 60),
        }
    }
}

/// Actual committed work, suitable for background cost attribution.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MaintenanceReport {
    pub files_rewritten_in: usize,
    pub files_rewritten_out: usize,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub snapshots_expired: usize,
    pub orphan_files_removed: usize,
    pub compaction_snapshot_committed: bool,
}

// Deterministic first-fit decreasing bin packing. A singleton is never rewritten.
fn groups(mut files: Vec<DataFile>, target: u64) -> Vec<Vec<DataFile>> {
    files.sort_by(|a, b| b.file_size_in_bytes().cmp(&a.file_size_in_bytes())
        .then_with(|| a.file_path().cmp(b.file_path())));
    let mut bins: Vec<(u64, Vec<DataFile>)> = Vec::new();
    for file in files {
        let size = file.file_size_in_bytes();
        if let Some((used, files)) = bins.iter_mut().find(|(used, _)| size <= target - *used) {
            *used += size;
            files.push(file);
        } else {
            bins.push((size, vec![file]));
        }
    }
    bins.into_iter().map(|(_, files)| files).filter(|files| files.len() >= 2).collect()
}

impl StorageCatalog {
    /// Runs one compaction pass. Retention controls are reserved and inactive.
    /// History and clone files are retained. Orphan deletion is not implemented.
    pub async fn maintain(&self, ident: &TableIdent, policy: &MaintenancePolicy) -> Result<MaintenanceReport> {
        if policy.target_file_size_bytes == 0 || policy.minimum_file_size_bytes == 0
            || policy.retain_last_snapshots == 0 {
            return Err("maintenance target, minimum size, and snapshot retention must be positive".into());
        }
        let table = self.catalog.load_table(ident).await?;
        let schema = writer::schema(&table)?;
        let files = writer::data_files(&table).await?;
        let tasks: Vec<_> = table.scan().build()?.plan_files().await?.try_collect().await?;
        let mut by_file: HashMap<String, Vec<FileScanTask>> = HashMap::new();
        for task in tasks {
            if !task.deletes().is_empty() { return Err("maintenance does not support delete files".into()); }
            by_file.entry(task.data_file_path().to_owned()).or_default().push(task);
        }
        let candidates = files.into_values().filter(|file|
            file.file_size_in_bytes() < policy.minimum_file_size_bytes
                && file.file_size_in_bytes() < policy.target_file_size_bytes as u64).collect();
        let mut removed = Vec::new();
        let mut added = Vec::new();
        for group in groups(candidates, policy.target_file_size_bytes as u64) {
            let mut batches = Vec::new();
            for file in &group {
                let tasks = by_file.remove(file.file_path()).ok_or("live file missing from maintenance scan")?;
                let read = writer::read_file(&table, tasks).await?;
                batches.extend(writer::normalize(&schema, read)?);
            }
            added.extend(writer::write_sized(&table, batches, policy.target_file_size_bytes).await?);
            removed.extend(group);
        }
        let mut report = MaintenanceReport::default();
        if !removed.is_empty() {
            report.files_rewritten_in = removed.len();
            report.files_rewritten_out = added.len();
            report.bytes_in = removed.iter().map(DataFile::file_size_in_bytes).sum();
            report.bytes_out = added.iter().map(DataFile::file_size_in_bytes).sum();
            self.commit_overwrite(&table, removed, added).await?;
            report.compaction_snapshot_committed = true;
        }
        Ok(report)
    }
}
