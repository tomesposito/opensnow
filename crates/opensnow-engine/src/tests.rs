use super::*;
use std::{sync::Arc, time::Duration};
use datafusion::{arrow::{array::Int64Array, datatypes::{DataType, Field, Schema}, record_batch::RecordBatch}, datasource::MemTable};

#[tokio::test]
async fn standing_yardstick_diff() { check().await.unwrap(); }

#[tokio::test]
async fn concurrency_isolation() {
    let mut service = EngineService::new();
    service.register_tenant("a", ResourceGroup { queue_capacity: 0, ..Default::default() }).unwrap();
    service.register_tenant("b", ResourceGroup { target_partitions: 3, ..Default::default() }).unwrap();
    assert_eq!(service.context("a").unwrap().state().config().target_partitions(), 2);
    assert_eq!(service.context("b").unwrap().state().config().target_partitions(), 3);
    let guard = service.acquire("a").await.unwrap();
    assert!(matches!(service.execute("a", "SELECT 1").await, Err(EngineError::AdmissionDenied)));
    assert_eq!(service.execute("b", "SELECT 1").await.unwrap().outcome.rows_returned, 1);
    drop(guard);
    service.execute("a", "SELECT 1").await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn queue_timeout_releases_capacity() {
    let mut service = EngineService::new();
    service.register_tenant("a", ResourceGroup { queue_capacity: 1, queue_timeout: Duration::from_secs(2), ..Default::default() }).unwrap();
    let guard = service.acquire("a").await.unwrap();
    assert!(matches!(service.acquire("a").await, Err(EngineError::AdmissionDenied)));
    let mut next = Box::pin(service.acquire("a"));
    assert!(futures::poll!(&mut next).is_pending());
    drop(guard);
    drop(next.await.unwrap());
    service.execute("a", "SELECT 1").await.unwrap();
}

#[tokio::test]
async fn cost_cap_isolation() {
    let mut service = EngineService::new();
    service.register_tenant("a", ResourceGroup { cost_cap_units: Some(2), ..Default::default() }).unwrap();
    service.register_tenant("b", ResourceGroup { cost_cap_units: Some(100), ..Default::default() }).unwrap();
    let sql = "SELECT * FROM (VALUES (1), (2), (3)) AS t(n)";
    assert!(matches!(service.execute("a", sql).await, Err(EngineError::CostCapExceeded)));
    assert_eq!(service.charged_cost("a").unwrap(), 3); // Aborting batch is charged.
    let guard = service.acquire("b").await.unwrap();
    // Invalid SQL proves the cap rejection happens before planning.
    assert!(matches!(service.execute("a", "invalid SQL").await, Err(EngineError::CostCapExceeded)));
    drop(guard);
    let result = service.execute("b", sql).await.unwrap();
    assert_eq!(result.outcome.cost_units_charged, 3);
    assert_eq!(result.outcome.rows_returned, 3);
    service.register_tenant("exact", ResourceGroup { cost_cap_units: Some(3), ..Default::default() }).unwrap();
    service.execute("exact", sql).await.unwrap();
    assert!(matches!(service.execute("exact", sql).await, Err(EngineError::CostCapExceeded)));
    assert!(matches!(service.execute("missing", sql).await, Err(EngineError::UnknownTenant)));
}

#[tokio::test]
async fn memory_pool_isolation() {
    let mut service = EngineService::new();
    service.register_tenant("low", ResourceGroup { memory_bytes: 1024, ..Default::default() }).unwrap();
    service.register_tenant("high", ResourceGroup { memory_bytes: 64 * 1024 * 1024, ..Default::default() }).unwrap();
    let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));
    let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(Int64Array::from_iter_values(0..4000))]).unwrap();
    for tenant in ["low", "high"] {
        service.context(tenant).unwrap().register_table("items", Arc::new(MemTable::try_new(schema.clone(), vec![vec![batch.clone()]]).unwrap())).unwrap();
    }
    // A non-equi nested-loop join retains its build side in a memory reservation.
    let sql = "SELECT count(*) FROM items a JOIN items b ON a.id < b.id";
    let result = service.execute("high", sql).await.unwrap();
    assert_eq!(result.outcome.rows_returned, 1);
    assert_eq!(datafusion::common::cast::as_int64_array(result.batches[0].column(0)).unwrap().value(0), 7_998_000);
    let error = service.execute("low", sql).await.unwrap_err();
    assert!(matches!(error, EngineError::Execution(_)), "{error}");
    assert!(error.to_string().to_lowercase().contains("resources exhausted"), "{error}");
    println!("PASS memory pool isolation: high completed, low returned {error}");
}

#[tokio::test]
async fn bounded_queue_and_cancellation() {
    let mut service = EngineService::new();
    service.register_tenant("a", ResourceGroup { queue_capacity: 1, ..Default::default() }).unwrap();
    service.register_tenant("b", ResourceGroup::default()).unwrap();
    let guard = service.acquire("a").await.unwrap();
    let mut waiter = Box::pin(service.acquire("a"));
    assert!(futures::poll!(&mut waiter).is_pending());
    assert!(matches!(service.acquire("a").await, Err(EngineError::AdmissionDenied)));
    service.execute("b", "SELECT 1").await.unwrap();
    drop(waiter); // Cancelling a queued request releases its queue capacity.
    let mut replacement = Box::pin(service.acquire("a"));
    assert!(futures::poll!(&mut replacement).is_pending());
    drop(guard);
    let guard = replacement.await.unwrap();
    drop(guard);
    service.execute("a", "SELECT 1").await.unwrap();
}

#[tokio::test]
async fn streamed_cost_abort_charges_processed_batches() {
    let mut service = EngineService::new();
    service.register_tenant("a", ResourceGroup { cost_cap_units: Some(3), ..Default::default() }).unwrap();
    let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));
    let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(Int64Array::from(vec![1, 2]))]).unwrap();
    service.context("a").unwrap().register_table("items", Arc::new(MemTable::try_new(schema, vec![vec![batch.clone(), batch.clone(), batch]]).unwrap())).unwrap();
    assert!(matches!(service.execute("a", "SELECT * FROM items").await, Err(EngineError::CostCapExceeded)));
    assert_eq!(service.charged_cost("a").unwrap(), 4); // Third batch never consumed.
}

#[tokio::test]
async fn unregister_removes_tenant() {
    let mut service = EngineService::new();
    service.register_tenant("a", ResourceGroup::default()).unwrap();
    assert!(service.is_registered("a"));
    service.execute("a", "SELECT 1").await.unwrap();
    service.unregister_tenant("a").unwrap();
    assert!(!service.is_registered("a"));
    assert!(matches!(service.execute("a", "SELECT 1").await, Err(EngineError::UnknownTenant)));
    assert!(matches!(service.unregister_tenant("a"), Err(EngineError::UnknownTenant)));
}

#[tokio::test]
async fn concurrent_queries_share_cumulative_cap() {
    let mut service = EngineService::new();
    service.register_tenant("a", ResourceGroup { max_concurrent: 2, cost_cap_units: Some(3), ..Default::default() }).unwrap();
    let sql = "SELECT * FROM (VALUES (1), (2)) AS t(n)";
    let (first, second) = tokio::join!(service.execute("a", sql), service.execute("a", sql));
    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    let error = first.err().or_else(|| second.err()).unwrap();
    assert!(matches!(error, EngineError::CostCapExceeded));
    assert_eq!(service.charged_cost("a").unwrap(), 4);
}
