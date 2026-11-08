//! Maintenance data preservation, metering, and history acceptance proofs.
use std::collections::BTreeMap;
use arrow::{array::{Array, Int64Array}, record_batch::RecordBatch};
use futures::TryStreamExt;
use iceberg::{spec::Operation, TableIdent};
use opensnow_storage::{CatalogConfig, MaintenancePolicy, MaintenanceReport, Result, StorageCatalog};

async fn setup() -> Result<(tempfile::TempDir, StorageCatalog, TableIdent)> {
    let temp = tempfile::tempdir()?;
    let storage = CatalogConfig::sqlite("maintenance", temp.path().join("catalog.db"), temp.path().join("warehouse"))?.connect().await?;
    let ident = TableIdent::from_strs(["db", "events"])?;
    storage.create_namespace(ident.namespace()).await?;
    storage.sql("CREATE TABLE ice.db.events (id BIGINT NOT NULL, value BIGINT)").await?;
    Ok((temp, storage, ident))
}

async fn append(storage: &StorageCatalog, start: i64, count: i64) -> Result<()> {
    let values = (start..start + count).map(|id| {
        let value = if id % 7 == 0 { "NULL".into() } else { (id * 3).to_string() };
        format!("({id}, {value})")
    }).collect::<Vec<_>>().join(",");
    storage.sql(&format!("INSERT INTO ice.db.events VALUES {values}")).await?;
    Ok(())
}

async fn files(storage: &StorageCatalog, ident: &TableIdent) -> Result<BTreeMap<String, u64>> {
    let table = storage.catalog.load_table(ident).await?;
    let tasks: Vec<_> = table.scan().build()?.plan_files().await?.try_collect().await?;
    let mut files = BTreeMap::new();
    for task in tasks {
        let path = task.data_file_path().to_owned();
        let size = std::fs::metadata(path.strip_prefix("file://").unwrap_or(&path))?.len();
        files.insert(path, size);
    }
    Ok(files)
}

fn rows(batches: &[RecordBatch]) -> Vec<(i64, Option<i64>)> {
    let mut rows = Vec::new();
    for batch in batches {
        let ids = batch.column(0).as_any().downcast_ref::<Int64Array>().unwrap();
        let values = batch.column(1).as_any().downcast_ref::<Int64Array>().unwrap();
        for i in 0..batch.num_rows() {
            rows.push((ids.value(i), (!values.is_null(i)).then(|| values.value(i))));
        }
    }
    rows.sort();
    rows
}

async fn live(storage: &StorageCatalog) -> Result<Vec<(i64, Option<i64>)>> {
    Ok(rows(&storage.sql("SELECT id, value FROM ice.db.events").await?))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fifty_appends_compact_preserving_rows_history_and_second_pass_noop() -> Result<()> {
    let (_temp, storage, ident) = setup().await?;
    for i in 0..50 { append(&storage, i * 10, 10).await?; }
    let before_files = files(&storage, &ident).await?;
    assert_eq!(before_files.len(), 50);
    let before = live(&storage).await?;
    assert_eq!(before.len(), 500);
    let snapshot = storage.current_snapshot_id(&ident).await?;
    let policy = MaintenancePolicy {
        target_file_size_bytes: 64 * 1024,
        minimum_file_size_bytes: before_files.values().max().unwrap() + 1,
        ..Default::default()
    };
    // All 50 input files fit one bin. Their merged output is one Parquet file.
    assert!(before_files.values().sum::<u64>() <= policy.target_file_size_bytes as u64);
    let report = storage.maintain(&ident, &policy).await?;
    let after = files(&storage, &ident).await?;
    assert_eq!(after.len(), 1);
    // Two BIGINT values give a 16-byte logical row slack for this fixture.
    assert!(after.values().all(|size| *size <= policy.target_file_size_bytes as u64 + 16));
    assert_eq!(report.files_rewritten_in, 50);
    assert_eq!(report.files_rewritten_out, 1);
    assert_eq!(report.bytes_in, before_files.values().sum::<u64>());
    assert_eq!(report.bytes_out, after.values().sum::<u64>());
    assert!(report.compaction_snapshot_committed);
    assert_eq!(report.snapshots_expired, 0);
    assert_eq!(report.orphan_files_removed, 0);
    assert_eq!(storage.snapshots(&ident).await?.len(), 51);
    let table = storage.catalog.load_table(&ident).await?;
    let current = table.metadata().current_snapshot().unwrap();
    assert_eq!(current.parent_snapshot_id(), Some(snapshot));
    assert_eq!(current.summary().operation, Operation::Overwrite);
    assert_eq!(live(&storage).await?, before);
    assert_eq!(rows(&storage.scan_snapshot(&ident, snapshot).await?), before);
    assert_eq!(storage.maintain(&ident, &policy).await?, MaintenanceReport::default());
    assert_eq!(files(&storage, &ident).await?, after);
    assert_eq!(storage.snapshots(&ident).await?.len(), 51);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn large_file_keeps_exact_path_while_small_files_merge() -> Result<()> {
    let (_temp, storage, ident) = setup().await?;
    append(&storage, 0, 5000).await?;
    let large = files(&storage, &ident).await?;
    let target = *large.values().next().unwrap() as usize;
    for i in 0..4 { append(&storage, 5000 + i * 10, 10).await?; }
    let before = live(&storage).await?;
    let report = storage.maintain(&ident, &MaintenancePolicy {
        target_file_size_bytes: target,
        minimum_file_size_bytes: target as u64,
        ..Default::default()
    }).await?;
    assert_eq!(report.files_rewritten_in, 4);
    assert_eq!(report.files_rewritten_out, 1);
    let after = files(&storage, &ident).await?;
    assert_eq!(after.len(), 2);
    for (path, size) in large { assert_eq!(after.get(&path), Some(&size)); }
    assert_eq!(live(&storage).await?, before);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disabled_expiry_preserves_all_history_and_clone_files() -> Result<()> {
    let (_temp, storage, ident) = setup().await?;
    append(&storage, 0, 10).await?;
    let original = live(&storage).await?;
    let clone = TableIdent::from_strs(["db", "clone"])?;
    storage.clone_table(&ident, &clone).await?;
    let clone_snapshot = storage.current_snapshot_id(&clone).await?;
    let clone_files = files(&storage, &clone).await?;
    storage.execute("DELETE FROM ice.db.events").await?;
    for i in 1..4 { append(&storage, i * 10, 10).await?; }
    let snapshots: Vec<_> = storage.snapshots(&ident).await?.into_iter().map(|s| s.id).collect();
    let report = storage.maintain(&ident, &MaintenancePolicy {
        minimum_file_size_bytes: 1,
        expire_before_ms: Some(i64::MAX),
        retain_last_snapshots: 1,
        orphan_grace_period: std::time::Duration::ZERO,
        ..Default::default()
    }).await?;
    assert_eq!(report, MaintenanceReport::default());
    assert_eq!(storage.snapshots(&ident).await?.into_iter().map(|s| s.id).collect::<Vec<_>>(), snapshots);
    assert_eq!(rows(&storage.scan_snapshot(&ident, clone_snapshot).await?), original);
    assert_eq!(rows(&storage.scan_snapshot(&clone, clone_snapshot).await?), original);
    assert_eq!(files(&storage, &clone).await?, clone_files);
    assert_eq!(live(&storage).await?.len(), 30);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn empty_singleton_invalid_policy_and_disabled_cleanup() -> Result<()> {
    let (_temp, storage, ident) = setup().await?;
    assert_eq!(storage.maintain(&ident, &MaintenancePolicy::default()).await?, MaintenanceReport::default());
    assert!(storage.snapshots(&ident).await?.is_empty());
    append(&storage, 0, 10).await?;
    let before = files(&storage, &ident).await?;
    let table = storage.catalog.load_table(&ident).await?;
    let data = std::path::Path::new(before.keys().next().unwrap()).parent().unwrap();
    let orphan = data.join("unreferenced.parquet");
    std::fs::write(&orphan, b"uncommitted")?;
    let policy = MaintenancePolicy { orphan_grace_period: std::time::Duration::ZERO, ..Default::default() };
    assert_eq!(storage.maintain(&ident, &policy).await?, MaintenanceReport::default());
    assert!(orphan.exists()); // Even zero grace cannot enable unsafe deletion.
    assert_eq!(files(&storage, &ident).await?, before);
    assert_eq!(storage.snapshots(&ident).await?.len(), 1);
    for policy in [MaintenancePolicy { target_file_size_bytes: 0, ..Default::default() }, MaintenancePolicy { minimum_file_size_bytes: 0, ..Default::default() }, MaintenancePolicy { retain_last_snapshots: 0, ..Default::default() }] {
        assert!(storage.maintain(&ident, &policy).await.is_err());
    }
    assert_eq!(table.metadata().current_snapshot_id(), Some(storage.current_snapshot_id(&ident).await?));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn multiple_bins_share_one_commit_and_singleton_keeps_path() -> Result<()> {
    let (_temp, storage, ident) = setup().await?;
    for i in 0..7 { append(&storage, i * 10, 10).await?; }
    let before = files(&storage, &ident).await?;
    let min = *before.values().min().unwrap();
    let max = *before.values().max().unwrap();
    let target = max * 2;
    assert!(min * 3 > target); // Exactly two files fit each bin.
    let original = live(&storage).await?;
    let snapshot = storage.current_snapshot_id(&ident).await?;
    let report = storage.maintain(&ident, &MaintenancePolicy {
        target_file_size_bytes: target as usize,
        minimum_file_size_bytes: max + 1,
        ..Default::default()
    }).await?;
    let after = files(&storage, &ident).await?;
    assert_eq!(report.files_rewritten_in, 6);
    assert_eq!(report.files_rewritten_out, 3);
    assert_eq!(after.len(), 4);
    assert_eq!(before.keys().filter(|path| after.contains_key(*path)).count(), 1);
    assert!(after.values().all(|size| *size <= target + 16));
    assert_eq!(storage.snapshots(&ident).await?.len(), 8);
    assert_eq!(live(&storage).await?, original);
    assert_eq!(rows(&storage.scan_snapshot(&ident, snapshot).await?), original);
    Ok(())
}

