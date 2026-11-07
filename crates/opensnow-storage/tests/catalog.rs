use std::collections::BTreeSet;
use arrow::{array::Int64Array, record_batch::RecordBatch};
use futures::StreamExt;
use iceberg::{NamespaceIdent, TableIdent};
use opensnow_storage::{CatalogConfig, Result, StorageCatalog};
use sqlx::ConnectOptions;

fn rows(batches: &[RecordBatch]) -> usize { batches.iter().map(RecordBatch::num_rows).sum() }
fn ids(batches: &[RecordBatch]) -> BTreeSet<i64> {
    batches.iter().flat_map(|b| b.column(0).as_any().downcast_ref::<Int64Array>().unwrap().values().to_vec()).collect()
}
async fn files(storage: &StorageCatalog, ident: &TableIdent) -> Result<BTreeSet<String>> {
    let table = storage.catalog.load_table(ident).await?;
    let scan = table.scan().build()?;
    let mut tasks = scan.plan_files().await?;
    let mut paths = BTreeSet::new();
    while let Some(task) = tasks.next().await { paths.insert(task?.data_file_path().to_owned()); }
    Ok(paths)
}
async fn append(storage: &StorageCatalog, start: i64, size: i64) -> Result<()> {
    let values = (start..start + size).map(|i| format!("({i})")).collect::<Vec<_>>().join(",");
    storage.sql(&format!("INSERT INTO ice.db.events VALUES {values}")).await?;
    Ok(())
}
async fn acceptance(storage: &StorageCatalog, first: i64, second: i64) -> Result<()> {
    let ns = NamespaceIdent::new("db".into());
    storage.create_namespace(&ns).await?;
    storage.sql("CREATE TABLE ice.db.events (id BIGINT NOT NULL)").await?;
    let source = TableIdent::new(ns.clone(), "events".into());
    append(storage, 0, first).await?;
    let first_id = storage.current_snapshot_id(&source).await?;
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    append(storage, first, second).await?;
    let total = first + second;
    let live = storage.sql("SELECT id FROM ice.db.events").await?;
    assert_eq!(rows(&live), total as usize);
    assert_eq!(ids(&live), (0..total).collect());
    let snapshots = storage.snapshots(&source).await?;
    assert_eq!(snapshots.len(), 2);
    assert!(snapshots[0].timestamp_ms < snapshots[1].timestamp_ms);
    assert_eq!(snapshots[0].id, first_id);
    let historical = storage.scan_snapshot(&source, first_id).await?;
    assert_eq!(rows(&historical), first as usize);
    assert_eq!(ids(&historical), (0..first).collect());
    let as_of = storage.scan_as_of(&source, snapshots[0].timestamp_ms).await?;
    assert_eq!(rows(&as_of), first as usize);
    assert_eq!(ids(&as_of), ids(&historical));
    assert!(storage.resolve_snapshot_as_of(&source, 0).await.unwrap_err().to_string().contains("no snapshot"));
    let target = TableIdent::new(ns, "cloned".into());
    let clone = storage.clone_table(&source, &target).await?;
    let original = storage.catalog.load_table(&source).await?;
    assert_eq!(original.metadata().format_version(), iceberg::spec::FormatVersion::V2);
    assert_ne!(clone.metadata().uuid(), original.metadata().uuid());
    assert_ne!(clone.metadata_location(), original.metadata_location());
    assert_eq!(clone.metadata().current_snapshot().unwrap().snapshot_id(), storage.current_snapshot_id(&source).await?);
    let paths = files(storage, &source).await?;
    assert!(!paths.is_empty());
    assert_eq!(files(storage, &target).await?, paths);
    assert_eq!(ids(&storage.sql("SELECT id FROM ice.db.cloned").await?), (0..total).collect());
    let clone_id = storage.current_snapshot_id(&target).await?;
    assert_eq!(rows(&storage.scan_snapshot(&target, clone_id).await?), total as usize);
    append(storage, total, 10).await?;
    assert_eq!(rows(&storage.sql("SELECT id FROM ice.db.events").await?), (total + 10) as usize);
    assert_eq!(rows(&storage.scan_snapshot(&target, storage.current_snapshot_id(&target).await?).await?), total as usize);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sqlite_round_trip_time_travel_and_clone() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let storage = CatalogConfig::sqlite("sqlite-test", temp.path().join("catalog.db"), temp.path().join("warehouse"))?.connect().await?;
    acceptance(&storage, 100, 50).await
}

/// SqlCatalog exposes no pool-close API. FORCE drops remaining idle pool sessions;
/// setup also drops leftovers from interrupted runs.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn postgres_round_trip_time_travel_and_clone() -> Result<()> {
    let mut admin = match sqlx::postgres::PgConnectOptions::new().host("localhost").port(5433).username("postgres").database("postgres")
        .connect().await {
        Ok(connection) => connection,
        Err(error) => {
            let unreachable = matches!(&error, sqlx::Error::Io(io) if matches!(io.kind(),
                std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::TimedOut |
                std::io::ErrorKind::NetworkUnreachable | std::io::ErrorKind::HostUnreachable));
            if unreachable {
                println!("SKIP Postgres catalog round trip: server unreachable: {error}");
                return Ok(());
            }
            return Err(error.into());
        }
    };
    sqlx::query("DROP DATABASE IF EXISTS opensnow_issue6_test WITH (FORCE)").execute(&mut admin).await?;
    sqlx::query("CREATE DATABASE opensnow_issue6_test").execute(&mut admin).await?;
    let temp = tempfile::tempdir()?;
    let result: Result<()> = async {
        let storage = CatalogConfig::postgres("postgres-test", "postgres://postgres@localhost:5433/opensnow_issue6_test", temp.path().join("warehouse")).connect().await?;
        acceptance(&storage, 3, 2).await
    }.await;
    let cleanup = sqlx::query("DROP DATABASE opensnow_issue6_test WITH (FORCE)").execute(&mut admin).await;
    result?;
    cleanup?;
    println!("PASS Postgres catalog round trip");
    Ok(())
}
