//! Regression evidence for the pinned provider's row mutation gap.
use arrow::{array::Int64Array, record_batch::RecordBatch};
use iceberg::TableIdent;
use opensnow_storage::{CatalogConfig, Result};

fn ids(batches: &[RecordBatch]) -> Vec<i64> {
    let mut ids: Vec<_> = batches.iter().flat_map(|batch| {
        batch.column(0).as_any().downcast_ref::<Int64Array>().unwrap().values().to_vec()
    }).collect();
    ids.sort();
    ids
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pinned_provider_rejects_row_mutations_without_changing_history() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let storage = CatalogConfig::sqlite("writer-blocker", temp.path().join("catalog.db"), temp.path().join("warehouse"))?.connect().await?;
    let ident = TableIdent::from_strs(["db", "events"])?;
    storage.create_namespace(ident.namespace()).await?;
    storage.sql("CREATE TABLE ice.db.events (id BIGINT NOT NULL)").await?;
    storage.sql("INSERT INTO ice.db.events VALUES (1), (2), (3)").await?;
    let before = storage.current_snapshot_id(&ident).await?;
    let history = storage.snapshots(&ident).await?;
    for sql in ["DELETE FROM ice.db.events WHERE id = 2", "UPDATE ice.db.events SET id = 20 WHERE id = 2"] {
        let error = storage.sql(sql).await.expect_err("pinned Iceberg provider must reject row mutations");
        let message = error.to_string().to_lowercase();
        assert!(message.contains("not implement") || message.contains("not support"), "{message}");
        assert_eq!(storage.snapshots(&ident).await?, history);
        assert_eq!(storage.current_snapshot_id(&ident).await?, before);
        assert_eq!(ids(&storage.sql("SELECT id FROM ice.db.events").await?), vec![1, 2, 3]);
        assert_eq!(ids(&storage.scan_snapshot(&ident, before).await?), vec![1, 2, 3]);
    }
    Ok(())
}
