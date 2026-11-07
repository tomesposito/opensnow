//! Content, file reuse, manifest metadata, and history proofs for copy-on-write.
use std::{collections::{BTreeMap, BTreeSet}};
use arrow::{array::{Array, Int64Array}, record_batch::RecordBatch};
use futures::TryStreamExt;
use iceberg::{
    spec::{DataContentType, DataFile, ManifestContentType, ManifestEntry, ManifestStatus, Operation},
    transaction::{ApplyTransactionAction, Transaction}, TableIdent,
};
use opensnow_storage::{CatalogConfig, Result, StorageCatalog};

struct Fixture {
    storage: StorageCatalog,
    ident: TableIdent,
    first: BTreeSet<String>,
    second: BTreeSet<String>,
    _temp: tempfile::TempDir,
}

impl Fixture {
    async fn new() -> Result<Self> {
        let temp = tempfile::tempdir()?;
        let storage = CatalogConfig::sqlite("writer", temp.path().join("catalog.db"), temp.path().join("warehouse"))?.connect().await?;
        let ident = TableIdent::from_strs(["db", "events"])?;
        storage.create_namespace(ident.namespace()).await?;
        storage.execute("CREATE TABLE ice.db.events (id BIGINT NOT NULL, value BIGINT)").await?;
        storage.execute("INSERT INTO ice.db.events VALUES (1, 10), (2, 20), (3, NULL)").await?;
        let first = files(&storage, &ident).await?;
        storage.execute("INSERT INTO ice.db.events VALUES (4, 40), (5, 50)").await?;
        let all = files(&storage, &ident).await?;
        let second = all.difference(&first).cloned().collect();
        assert_eq!(first.len(), 1);
        assert_eq!(all.len(), 2);
        Ok(Self { storage, ident, first, second, _temp: temp })
    }
    async fn live(&self) -> Result<Vec<(i64, Option<i64>)>> {
        Ok(rows(&self.storage.execute("SELECT id, value FROM ice.db.events").await?))
    }
    async fn history(&self) -> Result<Vec<i64>> {
        Ok(self.storage.snapshots(&self.ident).await?.into_iter().map(|s| s.id).collect())
    }
    async fn assert_snapshot(&self, previous: i64, expected: &[(i64, Option<i64>)], count: usize) -> Result<()> {
        assert_eq!(self.history().await?.len(), count);
        assert_eq!(rows(&self.storage.scan_snapshot(&self.ident, previous).await?), expected);
        let table = self.storage.catalog.load_table(&self.ident).await?;
        let snapshot = table.metadata().current_snapshot().unwrap();
        assert_eq!(snapshot.parent_snapshot_id(), Some(previous));
        assert_eq!(snapshot.summary().operation, Operation::Overwrite);
        let live = self.live().await?;
        let files = files(&self.storage, &self.ident).await?;
        let props = &snapshot.summary().additional_properties;
        assert_eq!(props["total-records"], live.len().to_string());
        assert_eq!(props["total-data-files"], files.len().to_string());
        assert_eq!(props["total-delete-files"], "0");
        let entries = entries(&self.storage, &self.ident).await?;
        let size: u64 = entries.values().filter(|e| e.is_alive()).map(|e| e.data_file().file_size_in_bytes()).sum();
        assert_eq!(props["total-files-size"], size.to_string());
        Ok(())
    }
}

fn rows(batches: &[RecordBatch]) -> Vec<(i64, Option<i64>)> {
    let mut rows: Vec<_> = batches.iter().flat_map(|batch| {
        let ids = batch.column(0).as_any().downcast_ref::<Int64Array>().unwrap();
        let values = batch.column(1).as_any().downcast_ref::<Int64Array>().unwrap();
        (0..batch.num_rows()).map(|i| (ids.value(i), (!values.is_null(i)).then(|| values.value(i)))).collect::<Vec<_>>()
    }).collect();
    rows.sort();
    rows
}

async fn files(storage: &StorageCatalog, ident: &TableIdent) -> Result<BTreeSet<String>> {
    let table = storage.catalog.load_table(ident).await?;
    let tasks: Vec<_> = table.scan().build()?.plan_files().await?.try_collect().await?;
    assert!(tasks.iter().all(|task| task.deletes().is_empty()));
    Ok(tasks.into_iter().map(|task| task.data_file_path().to_owned()).collect())
}

async fn entries(storage: &StorageCatalog, ident: &TableIdent) -> Result<BTreeMap<String, ManifestEntry>> {
    let table = storage.catalog.load_table(ident).await?;
    let list = table.manifest_list_reader(table.metadata().current_snapshot().unwrap()).load().await?;
    let mut entries = BTreeMap::new();
    for manifest_file in list.entries() {
        assert_eq!(manifest_file.content, ManifestContentType::Data);
        let manifest = table.manifest_reader().read(manifest_file).await?;
        for entry in manifest.entries() {
            assert_eq!(entry.data_file().content_type(), DataContentType::Data);
            entries.insert(entry.file_path().to_owned(), entry.as_ref().clone());
        }
    }
    Ok(entries)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_where_preserves_unaffected_files_and_history() -> Result<()> {
    let f = Fixture::new().await?;
    let original = f.live().await?;
    let mut history = f.history().await?;
    let before = *history.last().unwrap();
    f.storage.execute("dElEtE FROM ice.db.events WHERE id = 2").await?;
    assert_eq!(f.live().await?, vec![(1, Some(10)), (3, None), (4, Some(40)), (5, Some(50))]);
    let after = files(&f.storage, &f.ident).await?;
    assert!(f.second.is_subset(&after));
    assert!(f.first.is_disjoint(&after));
    f.assert_snapshot(before, &original, 3).await?;
    history.push(f.storage.current_snapshot_id(&f.ident).await?);
    assert_eq!(f.history().await?, history);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn update_where_preserves_unmatched_rows_and_files() -> Result<()> {
    let f = Fixture::new().await?;
    let original = f.live().await?;
    let before = f.storage.current_snapshot_id(&f.ident).await?;
    f.storage.execute("UPDATE ice.db.events SET value = value + 7, id = id + 10 WHERE id = 2").await?;
    assert_eq!(f.live().await?, vec![(1, Some(10)), (3, None), (4, Some(40)), (5, Some(50)), (12, Some(27))]);
    let after = files(&f.storage, &f.ident).await?;
    assert!(f.second.is_subset(&after));
    assert!(f.first.is_disjoint(&after));
    f.assert_snapshot(before, &original, 3).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_match_mutations_leave_snapshots_and_files_unchanged() -> Result<()> {
    let f = Fixture::new().await?;
    let history = f.history().await?;
    let paths = files(&f.storage, &f.ident).await?;
    let original = f.live().await?;
    for sql in ["DELETE FROM ice.db.events WHERE id = 99", "UPDATE ice.db.events SET value = 0 WHERE id = 99"] {
        f.storage.execute(sql).await?;
        assert_eq!(f.history().await?, history);
        assert_eq!(files(&f.storage, &f.ident).await?, paths);
        assert_eq!(f.live().await?, original);
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replace_all_swaps_rows_in_one_snapshot() -> Result<()> {
    let f = Fixture::new().await?;
    let before = f.storage.current_snapshot_id(&f.ident).await?;
    let old = f.live().await?;
    let batches = f.storage.sql("SELECT CAST(90 AS BIGINT) AS id, CAST(900 AS BIGINT) AS value").await?;
    f.storage.replace_all(&f.ident, batches).await?;
    assert_eq!(f.live().await?, vec![(90, Some(900))]);
    let paths = files(&f.storage, &f.ident).await?;
    assert!(paths.is_disjoint(&f.first));
    assert!(paths.is_disjoint(&f.second));
    f.assert_snapshot(before, &old, 3).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_delete_and_empty_replace_keep_table_queryable() -> Result<()> {
    let f = Fixture::new().await?;
    let before = f.storage.current_snapshot_id(&f.ident).await?;
    let original = f.live().await?;
    f.storage.execute("DELETE FROM ice.db.events").await?;
    assert!(f.live().await?.is_empty());
    assert!(files(&f.storage, &f.ident).await?.is_empty());
    f.assert_snapshot(before, &original, 3).await?;
    let history = f.history().await?;
    f.storage.execute("DELETE FROM ice.db.events").await?;
    f.storage.execute("UPDATE ice.db.events SET value = 1").await?;
    assert_eq!(f.history().await?, history);
    f.storage.replace_all(&f.ident, vec![]).await?;
    f.assert_snapshot(*history.last().unwrap(), &[], 4).await?;
    f.storage.execute("INSERT INTO ice.db.events VALUES (6, 60)").await?;
    assert_eq!(f.live().await?, vec![(6, Some(60))]);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn null_predicates_and_simultaneous_assignments_follow_sql() -> Result<()> {
    let f = Fixture::new().await?;
    let before = f.storage.current_snapshot_id(&f.ident).await?;
    let original = f.live().await?;
    f.storage.execute("DELETE FROM ice.db.events WHERE value < 20").await?;
    assert_eq!(f.live().await?, vec![(2, Some(20)), (3, None), (4, Some(40)), (5, Some(50))]);
    f.assert_snapshot(before, &original, 3).await?;
    f.storage.execute("UPDATE ice.db.events SET id = value, value = id WHERE value = 20").await?;
    assert_eq!(f.live().await?, vec![(3, None), (4, Some(40)), (5, Some(50)), (20, Some(2))]);
    f.storage.execute("UPDATE ice.db.events SET value = COALESCE(value, 0) + 1").await?;
    assert_eq!(f.live().await?, vec![(3, Some(1)), (4, Some(41)), (5, Some(51)), (20, Some(3))]);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rejected_sql_and_invalid_values_do_not_commit() -> Result<()> {
    let f = Fixture::new().await?;
    let before = f.history().await?;
    let original = f.live().await?;
    for sql in [
        "DELETE FROM events", "UPDATE events SET value = 0",
        "DELETE FROM other.db.events", "DELETE FROM ice.db.events RETURNING *",
        "DELETE FROM ice.db.events USING ice.db.events AS other",
        "DELETE FROM ice.db.events JOIN ice.db.events AS other ON true",
        "DELETE FROM (SELECT * FROM ice.db.events)",
        "DELETE FROM ice.db.events WHERE id IN (SELECT id FROM ice.db.events)",
        "UPDATE ice.db.events SET value = 1 FROM ice.db.events AS other",
        "UPDATE ice.db.events SET value = 1 RETURNING *",
        "UPDATE ice.db.events SET value = 1, value = 2",
        "UPDATE ice.db.events SET missing = 1 WHERE id = 99",
        "UPDATE ice.db.events SET value = missing WHERE id = 99",
        "UPDATE ice.db.events SET value = SUM(id)",
        "UPDATE ice.db.events SET value = random()",
        "UPDATE ice.db.events SET id = NULL WHERE id = 1",
        "UPDATE ice.db.events SET value = 'invalid' WHERE id = 1",
        "DELETE FROM ice.db.events WHERE 1",
        "DELETE FROM ice.db.events; DELETE FROM ice.db.events",
        "MERGE INTO ice.db.events USING ice.db.events AS other ON false WHEN MATCHED THEN DELETE",
    ] {
        assert!(f.storage.execute(sql).await.is_err(), "unexpectedly accepted: {sql}");
        assert_eq!(f.history().await?, before, "committed: {sql}");
        assert_eq!(f.live().await?, original, "changed rows: {sql}");
    }
    let wrong = f.storage.sql("SELECT 'invalid' AS id, 1 AS value").await?;
    assert!(f.storage.replace_all(&f.ident, wrong).await.is_err());
    assert_eq!(f.history().await?, before);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn empty_table_validates_expressions_and_accepts_replacement() -> Result<()> {
    let f = Fixture::new().await?;
    let empty = TableIdent::from_strs(["db", "empty"])?;
    f.storage.execute("CREATE TABLE ice.db.empty (id BIGINT NOT NULL, value BIGINT)").await?;
    assert!(f.storage.execute("UPDATE ice.db.empty SET value = missing").await.is_err());
    assert!(f.storage.execute("DELETE FROM ice.db.empty WHERE missing = 1").await.is_err());
    f.storage.execute("DELETE FROM ice.db.empty").await?;
    assert!(f.storage.snapshots(&empty).await?.is_empty());
    f.storage.replace_all(&empty, vec![]).await?;
    assert_eq!(f.storage.snapshots(&empty).await?.len(), 1);
    let batches = f.storage.sql("SELECT CAST(1 AS BIGINT) AS id, CAST(2 AS BIGINT) AS value").await?;
    f.storage.replace_all(&empty, batches).await?;
    assert_eq!(rows(&f.storage.sql("SELECT * FROM ice.db.empty").await?), vec![(1, Some(2))]);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manifest_rewrite_preserves_existing_entry_identity() -> Result<()> {
    let f = Fixture::new().await?;
    // Put both source files in one manifest to exercise the kept-entry branch.
    let target = TableIdent::from_strs(["db", "mixed"])?;
    f.storage.sql("CREATE TABLE ice.db.mixed (id BIGINT NOT NULL, value BIGINT)").await?;
    let data: Vec<DataFile> = entries(&f.storage, &f.ident).await?.into_values().map(|e| e.data_file().clone()).collect();
    let table = f.storage.catalog.load_table(&target).await?;
    let tx = Transaction::new(&table);
    tx.fast_append().add_data_files(data).apply(tx)?.commit(f.storage.catalog.as_ref()).await?;
    f.storage.refresh().await?;
    let original = entries(&f.storage, &target).await?;
    let before = f.storage.current_snapshot_id(&target).await?;
    f.storage.execute("DELETE FROM ice.db.mixed WHERE id <= 3").await?;
    let current = entries(&f.storage, &target).await?;
    for path in &f.second {
        let old = &original[path];
        let kept = &current[path];
        assert_eq!(kept.status(), ManifestStatus::Existing);
        assert_eq!(kept.snapshot_id(), old.snapshot_id());
        assert_eq!(kept.sequence_number(), old.sequence_number());
        assert_eq!(kept.file_sequence_number, old.file_sequence_number);
        assert_eq!(kept.data_file(), old.data_file());
    }
    for path in &f.first {
        let deleted = &current[path];
        assert_eq!(deleted.status(), ManifestStatus::Deleted);
        assert_eq!(deleted.snapshot_id(), Some(f.storage.current_snapshot_id(&target).await?));
        assert_eq!(deleted.sequence_number(), original[path].sequence_number());
        assert_eq!(deleted.file_sequence_number, original[path].file_sequence_number);
    }
    assert_eq!(rows(&f.storage.sql("SELECT * FROM ice.db.mixed").await?), vec![(4, Some(40)), (5, Some(50))]);
    assert_eq!(rows(&f.storage.scan_snapshot(&target, before).await?), f.live().await?);
    // Rewriting that manifest again must never resurrect its old deleted entries.
    f.storage.execute("UPDATE ice.db.mixed SET value = value + 1 WHERE id = 4").await?;
    assert_eq!(rows(&f.storage.sql("SELECT * FROM ice.db.mixed").await?), vec![(4, Some(41)), (5, Some(50))]);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn overwrite_rejects_duplicate_additions_and_stale_removals() -> Result<()> {
    let f = Fixture::new().await?;
    let table = f.storage.catalog.load_table(&f.ident).await?;
    let removed = entries(&f.storage, &f.ident).await?.into_values().next().unwrap().data_file().clone();
    let before = f.history().await?;
    let tx = Transaction::new(&table);
    assert!(tx.overwrite().add_data_files([removed.clone()]).apply(tx)?.commit(f.storage.catalog.as_ref()).await.is_err());
    assert_eq!(f.history().await?, before);
    let stale = Transaction::new(&table);
    let stale = stale.overwrite().delete_data_files([removed.clone()]).apply(stale)?;
    let tx = Transaction::new(&table);
    tx.overwrite().delete_data_files([removed.clone(), removed]).apply(tx)?.commit(f.storage.catalog.as_ref()).await?;
    let after = f.history().await?;
    assert_eq!(after.len(), before.len() + 1);
    assert!(stale.commit(f.storage.catalog.as_ref()).await.is_err());
    assert_eq!(f.history().await?, after);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn update_casts_assignments_without_coercing_unmatched_values() -> Result<()> {
    let f = Fixture::new().await?;
    let batches = f.storage.sql(
        "SELECT CAST(id AS BIGINT) AS id, CAST(value AS BIGINT) AS value FROM (VALUES (1, 10), (2, 9007199254740993)) AS input(id, value)"
    ).await?;
    f.storage.replace_all(&f.ident, batches).await?;
    let before = f.storage.current_snapshot_id(&f.ident).await?;
    let original = f.live().await?;
    f.storage.execute("/* precision */ UPDATE ICE.DB.EVENTS SET \"value\" = 1.5 WHERE id = 1;").await?;
    assert_eq!(f.live().await?, vec![(1, Some(1)), (2, Some(9007199254740993))]);
    f.assert_snapshot(before, &original, 4).await
}
