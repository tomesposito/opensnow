//! Tenant SQL service and analytical compatibility checks. See docs/ENGINE-SERVICE.md.
mod service;
pub use service::*;
mod metering;
pub use metering::*;
mod yardstick;
#[cfg(test)]
mod tests;
use std::sync::Arc;
use datafusion::arrow::array::{ArrayRef, Float64Array};
use datafusion::arrow::datatypes::DataType;
use datafusion::common::cast::{as_float64_array, as_int64_array};
use datafusion::error::Result;
use datafusion::execution::memory_pool::GreedyMemoryPool;
use datafusion::execution::runtime_env::RuntimeEnvBuilder;
use datafusion::logical_expr::{create_udf, ColumnarValue, ScalarFunctionImplementation, ScalarUDF, Volatility};
use datafusion::prelude::*;

/// Each tenant receives its own context and 64 MiB memory pool.
pub fn session() -> Result<SessionContext> {
    session_with_limits(64 * 1024 * 1024, 2)
}

fn session_with_limits(memory_bytes: usize, target_partitions: usize) -> Result<SessionContext> {
    let runtime = RuntimeEnvBuilder::new()
        .with_memory_pool(Arc::new(GreedyMemoryPool::new(memory_bytes)))
        .build_arc()?;
    let ctx = SessionContext::new_with_config_rt(
        SessionConfig::new().with_target_partitions(target_partitions), runtime);
    ctx.register_udf(net_price_udf());
    Ok(ctx)
}

fn net_price_udf() -> ScalarUDF {
    let fun: ScalarFunctionImplementation = Arc::new(|args: &[ColumnarValue]| {
        let (q, p, d) = (&args[0], &args[1], &args[2]);
        let out: ArrayRef = match (q, p, d) {
            (ColumnarValue::Array(q), ColumnarValue::Array(p), ColumnarValue::Array(d)) => {
                let q = as_int64_array(q)?;
                let p = as_float64_array(p)?;
                let d = as_float64_array(d)?;
                let res: Float64Array = q
                    .iter()
                    .zip(p.iter())
                    .zip(d.iter())
                    .map(|((q, p), d)| match (q, p, d) {
                        (Some(q), Some(p), Some(d)) => Some(q as f64 * p * (1.0 - d)),
                        _ => None,
                    })
                    .collect();
                Arc::new(res)
            }
            _ => {
                return Err(datafusion::error::DataFusionError::Internal(
                    "net_price expects array args".into(),
                ))
            }
        };
        Ok(ColumnarValue::Array(out))
    });
    create_udf(
        "net_price",
        vec![DataType::Int64, DataType::Float64, DataType::Float64],
        DataType::Float64,
        Volatility::Immutable,
        fun,
    )
}


/// Run the six B1 shapes on generated rows, with no spike data dependency.
pub async fn check() -> Result<()> {
    let ctx = session()?;
    let mut expected = 0.0;
    let mut rows = Vec::new();
    for i in 0..24 {
        let quantity = (i % 4 + 1) as i64;
        let price = (i + 10) as f64;
        let discount = if i % 2 == 0 { 0.25 } else { 0.0 };
        expected += quantity as f64 * price * (1.0 - discount);
        rows.push(format!("({i}, DATE '2024-{:02}-01', {}, {}, '{}', {quantity}, {price}, {discount})",
            i % 3 + 1, i % 4, i % 2, if i % 2 == 0 { "West" } else { "East" }));
    }
    ctx.sql(&format!("CREATE VIEW sales AS SELECT * FROM (VALUES {}) AS t(order_id, order_date, customer_id, product_id, region, quantity, unit_price, discount)", rows.join(","))).await?.collect().await?;
    ctx.sql("CREATE VIEW products AS SELECT * FROM (VALUES (0, 'hardware'), (1, 'software')) AS t(product_id, category)").await?.collect().await?;
    let expected_rows = [1, 6, 2, 10, 12, 2];
    for (i, sql) in include_str!("queries.sql").lines().filter(|s| !s.starts_with("--") && !s.trim().is_empty()).enumerate() {
        let batches = ctx.sql(sql).await?.collect().await?;
        let count: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(count, expected_rows[i], "B1 Q{} row count", i + 1);
        println!("PASS B1 Q{}: {count} rows", i + 1);
    }
    let batches = ctx.sql("SELECT sum(net_price(CAST(quantity AS BIGINT), CAST(unit_price AS DOUBLE), CAST(discount AS DOUBLE))) FROM sales").await?.collect().await?;
    let actual = as_float64_array(batches[0].column(0))?.value(0);
    assert!((actual - expected).abs() < 1e-9, "net revenue: {actual} != {expected}");
    println!("PASS B1 net_price: {actual} equals plain Rust {expected}");
    yardstick::check(&ctx).await?;
    println!("PASS B1 path: six analytical shapes and independent revenue check");
    Ok(())
}
