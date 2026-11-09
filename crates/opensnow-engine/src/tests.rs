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

#[test]
fn dollar_math_is_exact() {
    // Default rate from docs/COST-MODEL.md Section 1.
    assert_eq!(DEFAULT_VCPU_HOUR_DOLLARS, 0.0504);
    assert_eq!(cost_dollars(3600.0, DEFAULT_VCPU_HOUR_DOLLARS), 0.0504);
    assert_eq!(cost_dollars(0.0, DEFAULT_VCPU_HOUR_DOLLARS), 0.0);
    // A round test rate: $3600 per vCPU-hour is $1 per vCPU-second.
    assert_eq!(MeteringRates { vcpu_hour_dollars: 3600.0 }.cost_dollars(2.5), 2.5);
    assert_eq!(EngineService::new().metering_rates(), MeteringRates::default());
}

#[tokio::test]
async fn metering_records_completed_query() {
    let mut service = EngineService::with_metering_rates(MeteringRates { vcpu_hour_dollars: 3600.0 });
    service.register_tenant("a", ResourceGroup { target_partitions: 2, ..Default::default() }).unwrap();
    let sql = "SELECT * FROM (VALUES (1), (2), (3)) AS t(n)";
    let result = service.execute("a", sql).await.unwrap();
    let records = service.cost_records("a").unwrap();
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record.seq, 1);
    assert_eq!(record.kind, CostRecordKind::Query);
    assert_eq!(record.status, CostRecordStatus::Completed);
    assert_eq!(record.sql.as_deref(), Some(sql));
    assert_eq!(record.rows_returned, 3);
    let produced: u64 = result.batches.iter().map(|b| b.get_array_memory_size() as u64).sum();
    assert_eq!(record.bytes_returned, produced);
    assert!(record.bytes_returned > 0);
    assert_eq!(record.elapsed, result.outcome.elapsed);
    assert_eq!(record.vcpu_seconds, record.elapsed.as_secs_f64() * 2.0);
    assert_eq!(record.cost_dollars, record.vcpu_seconds); // $1 per vCPU-second at the test rate.
    assert_eq!(result.outcome.cost_record_seq, 1);
    assert_eq!(result.outcome.cost_dollars, record.cost_dollars);
    assert_eq!(result.outcome.bytes_returned, record.bytes_returned);
    assert!(record.started_at_unix_ms > 0);
    let summary = service.cost_summary("a").unwrap();
    assert_eq!(summary, TenantCostSummary::from_records(&records));
    assert_eq!(summary.record_count, 1);
    assert_eq!(summary.total_rows_returned, 3);
}

#[tokio::test]
async fn metering_records_failed_query() {
    let mut service = EngineService::new();
    service.register_tenant("a", ResourceGroup::default()).unwrap();
    let error = service.execute("a", "SELECT * FROM missing_table").await.unwrap_err();
    assert!(matches!(error, EngineError::Execution(_)));
    let records = service.cost_records("a").unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].status, CostRecordStatus::Failed);
    assert_eq!(records[0].rows_returned, 0);
    assert_eq!(records[0].bytes_returned, 0);
}

#[tokio::test]
async fn metering_records_cap_abort_with_charged_rows() {
    let mut service = EngineService::new();
    service.register_tenant("a", ResourceGroup { cost_cap_units: Some(3), ..Default::default() }).unwrap();
    let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));
    let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(Int64Array::from(vec![1, 2]))]).unwrap();
    service.context("a").unwrap().register_table("items", Arc::new(MemTable::try_new(schema, vec![vec![batch.clone(), batch.clone(), batch]]).unwrap())).unwrap();
    assert!(matches!(service.execute("a", "SELECT * FROM items").await, Err(EngineError::CostCapExceeded)));
    let records = service.cost_records("a").unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].status, CostRecordStatus::CostCapExceeded);
    // Same rows the existing accounting charges: two batches pulled, the
    // aborting one included, the third never pulled.
    assert_eq!(records[0].rows_returned, 4);
    assert_eq!(records[0].rows_returned, service.charged_cost("a").unwrap());
    // A pre-execution cap rejection executes nothing and records nothing.
    assert!(matches!(service.execute("a", "SELECT * FROM items").await, Err(EngineError::CostCapExceeded)));
    assert_eq!(service.cost_records("a").unwrap().len(), 1);
}

#[tokio::test]
async fn metering_isolated_per_tenant_and_background_records() {
    let mut service = EngineService::new();
    service.register_tenant("a", ResourceGroup::default()).unwrap();
    service.register_tenant("b", ResourceGroup::default()).unwrap();
    service.execute("a", "SELECT 1").await.unwrap();
    service.execute("a", "SELECT 2").await.unwrap();
    service.execute("b", "SELECT 1").await.unwrap();
    let background = service.record_background("a", "compaction", Duration::from_secs(2), 100, 4096).unwrap();
    assert_eq!(background.kind, CostRecordKind::Background);
    assert_eq!(background.service.as_deref(), Some("compaction"));
    assert_eq!(background.sql, None);
    assert_eq!(background.vcpu_seconds, 2.0 * 2.0); // elapsed x default target_partitions.
    let a_records = service.cost_records("a").unwrap();
    let b_records = service.cost_records("b").unwrap();
    assert_eq!(a_records.len(), 3);
    assert_eq!(b_records.len(), 1);
    assert_eq!(a_records[2], background);
    assert_eq!(service.cost_summary("a").unwrap(), TenantCostSummary::from_records(&a_records));
    assert_eq!(service.cost_summary("b").unwrap().total_rows_returned, 1);
    assert!(matches!(service.cost_records("missing"), Err(EngineError::UnknownTenant)));
    assert!(matches!(service.cost_summary("missing"), Err(EngineError::UnknownTenant)));
    assert!(matches!(service.record_background("missing", "compaction", Duration::ZERO, 0, 0), Err(EngineError::UnknownTenant)));
}
