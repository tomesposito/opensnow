// B1 spike: minimal query engine on Apache DataFusion.
// Modes: gen | bench | probe
use std::sync::Arc;
use std::time::Instant;

use datafusion::arrow::array::{ArrayRef, Float64Array, Int64Array};
use datafusion::arrow::datatypes::DataType;
use datafusion::common::cast::{as_float64_array, as_int64_array};
use datafusion::error::Result;
use datafusion::execution::memory_pool::GreedyMemoryPool;
use datafusion::execution::runtime_env::RuntimeEnvBuilder;
use datafusion::logical_expr::{create_udf, ColumnarValue, ScalarFunctionImplementation, ScalarUDF, Volatility};
use datafusion::prelude::*;

fn data_dir() -> String {
    let dir = std::env::var("B1_DATA").unwrap_or_else(|_| "data".to_string());
    format!("{}/{}", env!("CARGO_MANIFEST_DIR"), dir)
}

fn queries() -> Vec<(String, String)> {
    let raw = std::fs::read_to_string(format!("{}/queries.sql", env!("CARGO_MANIFEST_DIR")))
        .expect("queries.sql");
    let mut out = Vec::new();
    let mut label = String::new();
    for line in raw.lines() {
        let t = line.trim();
        if t.starts_with("--") {
            label = t.trim_start_matches('-').trim().to_string();
        } else if t.ends_with(';') && !t.is_empty() {
            out.push((label.clone(), t.to_string()));
        }
    }
    out
}

async fn register_parquet(ctx: &SessionContext) -> Result<()> {
    let d = data_dir();
    ctx.register_parquet("sales", &format!("{}/sales.parquet", d), ParquetReadOptions::default()).await?;
    ctx.register_parquet("products", &format!("{}/products.parquet", d), ParquetReadOptions::default()).await?;
    Ok(())
}

async fn gen() -> Result<()> {
    let ctx = SessionContext::new();
    let d = data_dir();
    ctx.register_csv("sales", &format!("{}/sales.csv", d), CsvReadOptions::new()).await?;
    ctx.register_csv("products", &format!("{}/products.csv", d), CsvReadOptions::new()).await?;
    for name in ["sales", "products"] {
        let sql = format!("COPY {} TO '{}/{}.parquet' STORED AS PARQUET", name, d, name);
        ctx.sql(&sql).await?.collect().await?;
        println!("wrote {}.parquet", name);
    }
    Ok(())
}

async fn bench() -> Result<()> {
    let ctx = SessionContext::new();
    register_parquet(&ctx).await?;
    for (label, sql) in queries() {
        let mut times = Vec::new();
        let mut rows = 0usize;
        for _ in 0..3 {
            let start = Instant::now();
            let batches = ctx.sql(&sql).await?.collect().await?;
            times.push(start.elapsed().as_secs_f64() * 1000.0);
            rows = batches.iter().map(|b| b.num_rows()).sum();
        }
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        println!("BENCH | {} | median_ms={:.1} | runs_ms={:?} | rows={}", label, times[1],
                 times.iter().map(|t| format!("{:.1}", t)).collect::<Vec<_>>(), rows);
    }
    Ok(())
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

async fn probe() -> Result<()> {
    // 1. Custom UDF on the default context
    let ctx = SessionContext::new();
    register_parquet(&ctx).await?;
    ctx.register_udf(net_price_udf());
    let start = Instant::now();
    let batches = ctx
        .sql("SELECT count(*) AS n, sum(net_price(quantity, unit_price, discount)) AS rev FROM sales")
        .await?
        .collect()
        .await?;
    println!("PROBE udf | ok | ms={:.1} | batches={}", start.elapsed().as_secs_f64() * 1000.0, batches.len());
    let pretty = datafusion::arrow::util::pretty::pretty_format_batches(&batches).unwrap().to_string();
    println!("PROBE udf result:\n{}", pretty);

    // 2. EXPLAIN quality (first lines only)
    let df = ctx.sql("EXPLAIN SELECT p.category, sum(s.quantity * s.unit_price) AS rev FROM sales s JOIN products p ON s.product_id = p.product_id GROUP BY p.category").await?;
    let batches = df.collect().await?;
    let pretty = datafusion::arrow::util::pretty::pretty_format_batches(&batches).unwrap().to_string();
    let lines: Vec<&str> = pretty.lines().take(28).collect();
    println!("PROBE explain (first {} lines):\n{}", lines.len(), lines.join("\n"));

    // 3. Per-tenant shape: a second context with its own limits
    let tenant_cfg = SessionConfig::new().with_target_partitions(2).with_batch_size(4096);
    let tenant_b = SessionContext::new_with_config(tenant_cfg);
    register_parquet(&tenant_b).await?;
    let (label, sql) = &queries()[1];
    let start = Instant::now();
    let batches = tenant_b.sql(sql).await?.collect().await?;
    println!("PROBE tenant_b (2 partitions, batch 4096) | {} | ms={:.1} | rows={}",
             label, start.elapsed().as_secs_f64() * 1000.0,
             batches.iter().map(|b| b.num_rows()).sum::<usize>());

    // 4. Memory pool + spill: full sort of 1M rows under a 64MB pool
    let pool = Arc::new(GreedyMemoryPool::new(64 * 1024 * 1024));
    let runtime = RuntimeEnvBuilder::new().with_memory_pool(pool).build_arc()?;
    let small = SessionContext::new_with_config_rt(SessionConfig::new(), runtime);
    register_parquet(&small).await?;
    let start = Instant::now();
    let res = small
        .sql("SELECT order_id, customer_id, region, quantity * unit_price AS rev FROM sales ORDER BY rev DESC")
        .await?
        .collect()
        .await;
    match res {
        Ok(batches) => println!("PROBE spill sort 64MB pool | ok | ms={:.1} | rows={}",
                                 start.elapsed().as_secs_f64() * 1000.0,
                                 batches.iter().map(|b| b.num_rows()).sum::<usize>()),
        Err(e) => println!("PROBE spill sort 64MB pool | ERROR | ms={:.1} | {}", start.elapsed().as_secs_f64() * 1000.0, e),
    }

    // 5. Same pool, big group by (hash agg is the classic spill gap)
    let start = Instant::now();
    let res = small
        .sql("SELECT customer_id, count(*) AS n, sum(quantity * unit_price) AS rev FROM sales GROUP BY customer_id ORDER BY rev DESC")
        .await?
        .collect()
        .await;
    match res {
        Ok(batches) => println!("PROBE spill groupby 64MB pool | ok | ms={:.1} | rows={}",
                                 start.elapsed().as_secs_f64() * 1000.0,
                                 batches.iter().map(|b| b.num_rows()).sum::<usize>()),
        Err(e) => println!("PROBE spill groupby 64MB pool | ERROR | ms={:.1} | {}", start.elapsed().as_secs_f64() * 1000.0, e),
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "bench".to_string());
    match mode.as_str() {
        "gen" => gen().await,
        "bench" => bench().await,
        "probe" => probe().await,
        other => {
            eprintln!("unknown mode: {}", other);
            std::process::exit(2);
        }
    }
}
