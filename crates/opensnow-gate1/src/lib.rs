//! Gate 1 evaluation harness (slice 1.10, issue #13).
//!
//! Generates the shared reference dataset (docs/workloads/README.md) at a
//! stated scale divisor, loads it into Iceberg tables through
//! opensnow-storage, runs the six workload files' statements through
//! EngineService (opensnow-engine), times them, and checks aggregate
//! results against plain Rust accumulators built during generation (the
//! yardstick pattern from opensnow-engine). Dates are anchored to
//! 2026-11-09, the run date: CURRENT_DATE() and CURRENT_TIMESTAMP() in
//! the workload files are replaced by that date and noon that day.
//!
//! The harness measures local runtime only. Monthly cost conversion
//! lives in docs/GATE1-EVALUATION.md, with its arithmetic stated there.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};
use arrow::{
    array::{
        Date32Array, Float64Array, Int64Array, StringArray, TimestampMicrosecondArray,
    },
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use datafusion::{arrow::util::display::array_value_to_string, datasource::MemTable, prelude::*};
use iceberg::{NamespaceIdent, TableIdent};
use iceberg_datafusion::IcebergCatalogProvider;
use opensnow_engine::{EngineService, ResourceGroup};
use opensnow_storage::{CatalogConfig, StorageCatalog};

pub const ANCHOR_DATE: &str = "2026-11-09";
pub const ANCHOR_TS: &str = "2026-11-09 12:00:00";

const REGIONS: [&str; 4] = ["East", "West", "North", "South"];
const CATEGORIES: [&str; 5] = ["Software", "Hardware", "Services", "Media", "Other"];
const SEGMENTS: [&str; 4] = ["Consumer", "Small Business", "Enterprise", "Public Sector"];
const COUNTRIES: [&str; 6] = ["US", "CA", "GB", "DE", "FR", "AU"];
const BRANDS: [&str; 7] = ["Acme", "Northwind", "Contoso", "Fabrikam", "Globex", "Initech", "Umbrella"];
const EVENT_TYPES: [&str; 5] = ["view", "click", "cart", "purchase", "signup"];
const PAGES: [&str; 8] = ["/home", "/product", "/cart", "/checkout", "/search", "/pricing", "/docs", "/account"];

// --- Civil date math (Howard Hinnant's algorithms) ---

pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn month_key(day: i64) -> i64 {
    let (y, m, _) = civil_from_days(day);
    y * 12 + (m - 1)
}

pub fn anchor_day() -> i64 { days_from_civil(2026, 11, 9) }

/// splitmix64 finalizer. Fact columns derive from different bit ranges
/// of the mixed index, so customer, product, quantity, price, and
/// discount are not modular shadows of each other.
fn mix(idx: i64) -> u64 {
    let mut z = (idx as u64).wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

// --- Dataset ---

/// Row counts at full scale, from docs/workloads/README.md.
pub struct Scale {
    pub divisor: i64,
    pub sales: i64,
    pub clickstream: i64,
    pub orders_raw: i64,
    pub customers: i64,
    pub products: i64,
}

impl Scale {
    pub fn new(divisor: i64) -> Self {
        Self {
            divisor,
            sales: 2_000_000_000 / divisor,
            clickstream: 6_000_000_000 / divisor,
            orders_raw: 350_000_000 / divisor,
            customers: (2_000_000 / divisor).max(10),
            products: (500_000 / divisor).max(10),
        }
    }
}

/// Plain Rust aggregates over the generated rows. The yardstick.
#[derive(Default)]
pub struct Expected {
    pub sales_count: i64,
    pub sales_net: f64,
    /// W2.1: (region, month key) -> (count, net), rows on or after 2025-11-09.
    pub w21: HashMap<(String, i64), (i64, f64)>,
    /// W3.1: (category, segment) -> (count, net), rows on or after 2026-05-09.
    pub w31: HashMap<(String, String), (i64, f64)>,
    /// Month to date (order_date >= 2026-11-01): net, distinct customers.
    pub mtd_net: f64,
    pub mtd_customers: HashSet<i64>,
    pub mtd_east_net: f64,
    /// Customer 42 month to date: net, orders, distinct products.
    pub cust42_mtd_net: f64,
    pub cust42_mtd_orders: i64,
    pub cust42_mtd_products: HashSet<i64>,
    /// W1.3: distinct (day, region) pairs with day offset <= 35.
    pub w13_pairs: HashSet<(i64, String)>,
    /// W1.1: deduped orders_raw rows loaded in the last day.
    pub w11_deduped: i64,
    /// W2.5: event_type -> (events, distinct sessions), last 7 days.
    pub w25: HashMap<String, (i64, HashSet<i64>)>,
    pub distinct_customers_sales: HashSet<i64>,
}

pub struct Dataset {
    pub probe_customer: i64,
    pub scale: Scale,
    pub expected: Expected,
    pub sales: Vec<RecordBatch>,
    pub clickstream: Vec<RecordBatch>,
    pub orders_raw: Vec<RecordBatch>,
    pub dim_customer: Vec<RecordBatch>,
    pub dim_product: Vec<RecordBatch>,
}

fn sales_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("order_id", DataType::Int64, false),
        Field::new("order_date", DataType::Date32, false),
        Field::new("customer_id", DataType::Int64, false),
        Field::new("product_id", DataType::Int64, false),
        Field::new("region", DataType::Utf8, false),
        Field::new("quantity", DataType::Int64, false),
        Field::new("unit_price", DataType::Float64, false),
        Field::new("discount", DataType::Float64, false),
    ]))
}

fn raw_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("order_id", DataType::Int64, false),
        Field::new("order_date", DataType::Date32, false),
        Field::new("customer_id", DataType::Int64, false),
        Field::new("product_id", DataType::Int64, false),
        Field::new("region", DataType::Utf8, false),
        Field::new("quantity", DataType::Int64, false),
        Field::new("unit_price", DataType::Float64, false),
        Field::new("discount", DataType::Float64, false),
        Field::new("loaded_at", DataType::Timestamp(arrow::datatypes::TimeUnit::Microsecond, None), false),
    ]))
}

fn click_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("event_id", DataType::Int64, false),
        Field::new("session_id", DataType::Int64, false),
        Field::new("customer_id", DataType::Int64, false),
        Field::new("event_type", DataType::Utf8, false),
        Field::new("page", DataType::Utf8, false),
        Field::new("event_ts", DataType::Timestamp(arrow::datatypes::TimeUnit::Microsecond, None), false),
    ]))
}

fn customer_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("customer_id", DataType::Int64, false),
        Field::new("customer_name", DataType::Utf8, false),
        Field::new("segment", DataType::Utf8, false),
        Field::new("country", DataType::Utf8, false),
        Field::new("signup_date", DataType::Date32, false),
    ]))
}

fn product_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("product_id", DataType::Int64, false),
        Field::new("category", DataType::Utf8, false),
        Field::new("product_name", DataType::Utf8, false),
        Field::new("brand", DataType::Utf8, false),
    ]))
}

pub fn features_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("customer_id", DataType::Int64, false),
        Field::new("recency_days", DataType::Int64, true),
        Field::new("orders_12m", DataType::Int64, true),
        Field::new("spend_12m", DataType::Float64, true),
        Field::new("avg_order_value", DataType::Float64, true),
        Field::new("spend_percentile_in_segment", DataType::Float64, true),
        Field::new("spend_decile", DataType::Int64, true),
        Field::new("events_this_month", DataType::Int64, true),
        Field::new("activity_ratio", DataType::Float64, true),
    ]))
}

pub fn agg_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("order_date", DataType::Date32, false),
        Field::new("region", DataType::Utf8, false),
        Field::new("net_revenue", DataType::Float64, true),
        Field::new("order_count", DataType::Int64, true),
        Field::new("avg_daily_revenue", DataType::Float64, true),
    ]))
}

const BATCH: i64 = 100_000;

/// Deterministic generation. Every distribution is modular arithmetic on
/// the row index, so the Rust accumulators and the SQL results describe
/// exactly the same rows.
pub fn generate(scale: Scale) -> Dataset {
    let probe_customer = 42.min(scale.customers - 1);
    let anchor = anchor_day();
    let day_minus_12m = days_from_civil(2025, 11, 9);
    let day_minus_6m = days_from_civil(2026, 5, 9);
    let day_mtd = days_from_civil(2026, 11, 1);
    let anchor_ts = anchor * 86_400;
    let noon = anchor_ts + 43_200;
    let mut exp = Expected::default();

    // sales_fact
    let mut sales = Vec::new();
    let mut i = 0i64;
    while i < scale.sales {
        let n = BATCH.min(scale.sales - i);
        let (mut ids, mut dates, mut custs, mut prods, mut regs, mut qtys, mut prices, mut discs) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for k in 0..n {
            let idx = i + k;
            let day_off = idx % 720;
            let day = anchor - day_off;
            let z = mix(idx);
            let customer = ((z >> 11) % scale.customers as u64) as i64;
            let product = ((z >> 32) % scale.products as u64) as i64;
            let region = REGIONS[(idx % 4) as usize];
            let qty = (z % 5) as i64 + 1;
            let price = 10.0 + ((z >> 48) % 9000) as f64 / 100.0;
            let disc = ((z >> 58) % 11) as f64 * 0.05;
            let net = qty as f64 * price * (1.0 - disc);
            exp.sales_count += 1;
            exp.sales_net += net;
            exp.distinct_customers_sales.insert(customer);
            if day >= day_minus_12m {
                let e = exp.w21.entry((region.to_string(), month_key(day))).or_default();
                e.0 += 1; e.1 += net;
            }
            if day >= day_minus_6m {
                let cat = CATEGORIES[(product % 5) as usize];
                let seg = SEGMENTS[(customer % 4) as usize];
                let e = exp.w31.entry((cat.to_string(), seg.to_string())).or_default();
                e.0 += 1; e.1 += net;
            }
            if day >= day_mtd {
                exp.mtd_net += net;
                exp.mtd_customers.insert(customer);
                if region == "East" { exp.mtd_east_net += net; }
                if customer == probe_customer {
                    exp.cust42_mtd_net += net;
                    exp.cust42_mtd_orders += 1;
                    exp.cust42_mtd_products.insert(product);
                }
            }
            if day_off <= 35 { exp.w13_pairs.insert((day, region.to_string())); }
            ids.push(idx + 1);
            dates.push(day as i32);
            custs.push(customer);
            prods.push(product);
            regs.push(region);
            qtys.push(qty);
            prices.push(price);
            discs.push(disc);
        }
        sales.push(RecordBatch::try_new(sales_schema(), vec![
            Arc::new(Int64Array::from(ids)),
            Arc::new(Date32Array::from(dates)),
            Arc::new(Int64Array::from(custs)),
            Arc::new(Int64Array::from(prods)),
            Arc::new(StringArray::from(regs)),
            Arc::new(Int64Array::from(qtys)),
            Arc::new(Float64Array::from(prices)),
            Arc::new(Float64Array::from(discs)),
        ]).unwrap());
        i += n;
    }

    // clickstream_events: the last 30 days.
    let mut clickstream = Vec::new();
    let mut i = 0i64;
    while i < scale.clickstream {
        let n = BATCH.min(scale.clickstream - i);
        let (mut ids, mut sessions, mut custs, mut types, mut pages, mut tss) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for k in 0..n {
            let idx = i + k;
            let z = mix(idx ^ 0x51AB);
            let ts = anchor_ts - (idx % 30) * 86_400 + ((z >> 20) % 86_400) as i64;
            let session = ((z >> 8) % (scale.customers as u64 * 10)) as i64;
            let etype = EVENT_TYPES[(idx % 5) as usize];
            if ts >= noon - 7 * 86_400 {
                let e = exp.w25.entry(etype.to_string()).or_default();
                e.0 += 1; e.1.insert(session);
            }
            ids.push(idx + 1);
            sessions.push(session);
            let ts = ts * 1_000_000;
            custs.push((z % scale.customers as u64) as i64);
            types.push(etype);
            pages.push(PAGES[(idx % 8) as usize]);
            tss.push(ts);
        }
        clickstream.push(RecordBatch::try_new(click_schema(), vec![
            Arc::new(Int64Array::from(ids)),
            Arc::new(Int64Array::from(sessions)),
            Arc::new(Int64Array::from(custs)),
            Arc::new(StringArray::from(types)),
            Arc::new(StringArray::from(pages)),
            Arc::new(TimestampMicrosecondArray::from(tss)),
        ]).unwrap());
        i += n;
    }

    // orders_raw: 7 days retained, loaded over the last 7 days. Every
    // 100th row duplicates the previous row's order_id with a later
    // loaded_at, the retried-extract shape W1 dedupes.
    let mut orders_raw = Vec::new();
    let mut latest: HashMap<i64, i64> = HashMap::new();
    let mut i = 0i64;
    while i < scale.orders_raw {
        let n = BATCH.min(scale.orders_raw - i);
        let (mut ids, mut dates, mut custs, mut prods, mut regs, mut qtys, mut prices, mut discs, mut loads) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for k in 0..n {
            let idx = i + k;
            let order_id = if idx % 100 == 99 && idx > 0 { idx } else { idx + 1 };
            let loaded = noon - (idx % 7) * 86_400 + (idx % 3600) + if idx % 100 == 99 { 3600 } else { 0 };
            if loaded >= noon - 86_400 {
                let e = latest.entry(order_id).or_insert(i64::MIN);
                if loaded > *e { *e = loaded; }
            }
            let z = mix(idx ^ 0x77CD);
            ids.push(order_id);
            dates.push((anchor - (idx % 30)) as i32);
            custs.push(((z >> 11) % scale.customers as u64) as i64);
            prods.push(((z >> 32) % scale.products as u64) as i64);
            regs.push(REGIONS[(idx % 4) as usize]);
            qtys.push((z % 5) as i64 + 1);
            prices.push(10.0 + ((z >> 48) % 9000) as f64 / 100.0);
            discs.push(((z >> 58) % 11) as f64 * 0.05);
            loads.push(loaded * 1_000_000);
        }
        orders_raw.push(RecordBatch::try_new(raw_schema(), vec![
            Arc::new(Int64Array::from(ids)),
            Arc::new(Date32Array::from(dates)),
            Arc::new(Int64Array::from(custs)),
            Arc::new(Int64Array::from(prods)),
            Arc::new(StringArray::from(regs)),
            Arc::new(Int64Array::from(qtys)),
            Arc::new(Float64Array::from(prices)),
            Arc::new(Float64Array::from(discs)),
            Arc::new(TimestampMicrosecondArray::from(loads)),
        ]).unwrap());
        i += n;
    }
    exp.w11_deduped = latest.len() as i64;

    // Dimensions, generated at the scaled counts.
    let dim_customer = vec![RecordBatch::try_new(customer_schema(), vec![
        Arc::new(Int64Array::from_iter_values(0..scale.customers)),
        Arc::new(StringArray::from_iter_values((0..scale.customers).map(|c| format!("customer-{c}")))),
        Arc::new(StringArray::from_iter_values((0..scale.customers).map(|c| SEGMENTS[(c % 4) as usize]))),
        Arc::new(StringArray::from_iter_values((0..scale.customers).map(|c| COUNTRIES[(c % 6) as usize]))),
        Arc::new(Date32Array::from_iter_values((0..scale.customers).map(|c| (anchor - (c % 1000)) as i32))),
    ]).unwrap()];
    let dim_product = vec![RecordBatch::try_new(product_schema(), vec![
        Arc::new(Int64Array::from_iter_values(0..scale.products)),
        Arc::new(StringArray::from_iter_values((0..scale.products).map(|p| CATEGORIES[(p % 5) as usize]))),
        Arc::new(StringArray::from_iter_values((0..scale.products).map(|p| format!("product-{p}")))),
        Arc::new(StringArray::from_iter_values((0..scale.products).map(|p| BRANDS[(p % 7) as usize]))),
    ]).unwrap()];

    Dataset { probe_customer, scale, expected: exp, sales, clickstream, orders_raw, dim_customer, dim_product }
}

// --- Statement ledger ---

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ledger {
    /// Runs as written, apart from the anchored CURRENT_DATE /
    /// CURRENT_TIMESTAMP substitution and the ice.db table prefix.
    AsWritten,
    /// Runs after the named rewrite.
    Rewrite(&'static str),
    /// The prototype cannot run it. The reason is the finding.
    CouldNotRun(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check {
    None,
    MtdEast,
    GlobalTotals,
    DedupedCount,
    DailyAggRows,
    MergeSales,
    MergeAgg,
    MergeFeatures,
    UpdateFromRecency,
    Zero,
    W21Full,
    W21West,
    MtdTotals,
    Cust42Mtd,
    Funnel,
    W31Full,
    DistinctCustomers,
}

pub struct Stmt {
    pub id: &'static str,
    pub workload: &'static str,
    pub ledger: Ledger,
    pub sql: Option<String>,
    pub check: Check,
    /// Tables a full scan of this statement touches, for the request model.
    pub scans: &'static [&'static str],
}

const DEDUPED: &str = "SELECT order_id, order_date, customer_id, product_id, region, quantity, unit_price, discount FROM ice.db.orders_raw WHERE loaded_at >= TIMESTAMP '2026-11-08 12:00:00' QUALIFY ROW_NUMBER() OVER (PARTITION BY order_id ORDER BY loaded_at DESC) = 1";
const FEATURES_BASE: &str = "SELECT customer_id, CAST((EXTRACT(EPOCH FROM DATE '2026-11-09') - EXTRACT(EPOCH FROM MAX(order_date))) / 86400 AS BIGINT) AS recency_days, COUNT(*) AS orders_12m, SUM(quantity * unit_price * (1 - discount)) AS spend_12m, AVG(quantity * unit_price * (1 - discount)) AS avg_order_value FROM ice.db.sales_fact WHERE order_date >= DATE '2026-11-09' - INTERVAL '12' MONTH GROUP BY customer_id";
const FEATURES_RANKED: &str = "SELECT b.customer_id, b.recency_days, b.orders_12m, b.spend_12m, b.avg_order_value, PERCENT_RANK() OVER (PARTITION BY c.segment ORDER BY b.spend_12m) AS spend_percentile_in_segment, CAST(NTILE(10) OVER (ORDER BY b.spend_12m DESC) AS BIGINT) AS spend_decile FROM (SELECT customer_id, CAST((EXTRACT(EPOCH FROM DATE '2026-11-09') - EXTRACT(EPOCH FROM MAX(order_date))) / 86400 AS BIGINT) AS recency_days, COUNT(*) AS orders_12m, SUM(quantity * unit_price * (1 - discount)) AS spend_12m, AVG(quantity * unit_price * (1 - discount)) AS avg_order_value FROM ice.db.sales_fact WHERE order_date >= DATE '2026-11-09' - INTERVAL '12' MONTH GROUP BY customer_id) AS b JOIN ice.db.dim_customer AS c ON c.customer_id = b.customer_id";
const FEATURES_TREND: &str = "WITH monthly AS (SELECT customer_id, date_trunc('month', event_ts) AS month, COUNT(*) AS events FROM ice.db.clickstream_events WHERE event_ts >= TIMESTAMP '2026-11-09 12:00:00' - INTERVAL '13' MONTH GROUP BY customer_id, month), trailing AS (SELECT customer_id, AVG(events) AS avg_monthly_events FROM monthly WHERE month < date_trunc('month', TIMESTAMP '2026-11-09 12:00:00') GROUP BY customer_id) SELECT m.customer_id, m.events AS events_this_month, t.avg_monthly_events, m.events / NULLIF(t.avg_monthly_events, 0) AS activity_ratio FROM monthly AS m JOIN trailing AS t ON t.customer_id = m.customer_id WHERE m.month = date_trunc('month', TIMESTAMP '2026-11-09 12:00:00')";

pub fn statements(probe_customer: i64) -> Vec<Stmt> {
    let cust = probe_customer;
    let s = |id, workload, ledger, sql: Option<String>, check, scans| Stmt { id, workload, ledger, sql, check, scans };
    vec![
        // W1 Nightly ELT
        s("W1.1", "W1", Ledger::Rewrite("CREATE OR REPLACE TEMP TABLE wrapper dropped; the dedupe SELECT body runs as written (QUALIFY is SF01, runs as written)"),
          Some(DEDUPED.into()), Check::DedupedCount, &["orders_raw"]),
        s("W1.2", "W1", Ledger::Rewrite("MERGE runs through the storage writer (issue #18); the orders_deduped temp table is inlined as the merge source, as in W1.4b"),
          Some(format!("MERGE INTO ice.db.sales_fact AS f USING ({DEDUPED}) AS o ON f.order_id = o.order_id WHEN MATCHED THEN UPDATE SET f.order_date = o.order_date, f.customer_id = o.customer_id, f.product_id = o.product_id, f.region = o.region, f.quantity = o.quantity, f.unit_price = o.unit_price, f.discount = o.discount WHEN NOT MATCHED THEN INSERT (order_id, order_date, customer_id, product_id, region, quantity, unit_price, discount) VALUES (o.order_id, o.order_date, o.customer_id, o.product_id, o.region, o.quantity, o.unit_price, o.discount)")),
          Check::MergeSales, &["sales_fact", "orders_raw"]),
        s("W1.3", "W1", Ledger::Rewrite("DATEADD rewritten as date minus INTERVAL (docs/COMPATIBILITY.md SF04); TEMP TABLE wrapper dropped, the rebuild SELECT runs"),
          Some("WITH daily AS (SELECT order_date, region, SUM(quantity * unit_price * (1 - discount)) AS net_revenue, COUNT(*) AS order_count FROM ice.db.sales_fact WHERE order_date >= DATE '2026-11-09' - INTERVAL '35' DAY GROUP BY order_date, region), monthly AS (SELECT date_trunc('month', order_date) AS month, region, AVG(net_revenue) AS avg_daily_revenue FROM daily GROUP BY month, region) SELECT d.order_date, d.region, d.net_revenue, d.order_count, m.avg_daily_revenue FROM daily AS d JOIN monthly AS m ON m.month = date_trunc('month', d.order_date) AND m.region = d.region".into()),
          Check::DailyAggRows, &["sales_fact"]),
        s("W1.3b", "W1", Ledger::Rewrite("MERGE runs through the storage writer (issue #18); the daily_revenue_new temp table is the W1.3 SELECT inlined as the merge source"),
          Some("MERGE INTO ice.db.daily_revenue_agg AS a USING (WITH daily AS (SELECT order_date, region, SUM(quantity * unit_price * (1 - discount)) AS net_revenue, COUNT(*) AS order_count FROM ice.db.sales_fact WHERE order_date >= DATE '2026-11-09' - INTERVAL '35' DAY GROUP BY order_date, region), monthly AS (SELECT date_trunc('month', order_date) AS month, region, AVG(net_revenue) AS avg_daily_revenue FROM daily GROUP BY month, region) SELECT d.order_date, d.region, d.net_revenue, d.order_count, m.avg_daily_revenue FROM daily AS d JOIN monthly AS m ON m.month = date_trunc('month', d.order_date) AND m.region = d.region) AS n ON a.order_date = n.order_date AND a.region = n.region WHEN MATCHED THEN UPDATE SET a.net_revenue = n.net_revenue, a.order_count = n.order_count, a.avg_daily_revenue = n.avg_daily_revenue WHEN NOT MATCHED THEN INSERT (order_date, region, net_revenue, order_count, avg_daily_revenue) VALUES (n.order_date, n.region, n.net_revenue, n.order_count, n.avg_daily_revenue)".into()),
          Check::MergeAgg, &["daily_revenue_agg", "sales_fact"]),
        s("W1.4a", "W1", Ledger::AsWritten,
          Some("SELECT COUNT(*) AS null_order_ids FROM ice.db.sales_fact WHERE order_id IS NULL AND order_date >= DATE '2026-11-08'".into()), Check::Zero, &["sales_fact"]),
        s("W1.4b", "W1", Ledger::Rewrite("orders_deduped temp table replaced by the inline dedupe subquery"),
          Some(format!("SELECT COUNT(*) AS negative_revenue_rows FROM ({DEDUPED}) AS d WHERE quantity * unit_price * (1 - discount) < 0")), Check::Zero, &["orders_raw"]),
        s("W1.4c", "W1", Ledger::Rewrite("orders_deduped temp table replaced by the inline dedupe subquery"),
          Some(format!("SELECT COUNT(*) AS orphan_customers FROM ({DEDUPED}) AS o LEFT JOIN ice.db.dim_customer AS c ON c.customer_id = o.customer_id WHERE c.customer_id IS NULL")), Check::Zero, &["orders_raw", "dim_customer"]),
        // W2 BI dashboards
        s("W2.1", "W2", Ledger::Rewrite("DATEADD rewritten as date minus INTERVAL (SF04); grouped month cast to DATE (DataFusion date_trunc returns a timestamp)"),
          Some("SELECT region, CAST(date_trunc('month', order_date) AS DATE) AS month, SUM(quantity * unit_price * (1 - discount)) AS net_revenue, COUNT(*) AS order_count FROM ice.db.sales_fact WHERE order_date >= DATE '2026-11-09' - INTERVAL '12' MONTH GROUP BY region, month ORDER BY region, month".into()),
          Check::W21Full, &["sales_fact"]),
        s("W2.2", "W2", Ledger::Rewrite("DATEADD rewritten as date minus INTERVAL (SF04); grouped month cast to DATE (DataFusion date_trunc returns a timestamp)"),
          Some("SELECT region, CAST(date_trunc('month', order_date) AS DATE) AS month, SUM(quantity * unit_price * (1 - discount)) AS net_revenue, COUNT(*) AS order_count FROM ice.db.sales_fact WHERE order_date >= DATE '2026-11-09' - INTERVAL '12' MONTH AND region = 'West' GROUP BY region, month ORDER BY region, month".into()),
          Check::W21West, &["sales_fact"]),
        s("W2.3", "W2", Ledger::Rewrite("DATE_TRUNC bound cast to DATE: DataFusion date_trunc on a date returns a nanosecond timestamp, which the Iceberg provider cannot push into a date column"),
          Some("SELECT p.product_name, p.category, SUM(f.quantity * f.unit_price * (1 - f.discount)) AS net_revenue FROM ice.db.sales_fact AS f JOIN ice.db.dim_product AS p ON p.product_id = f.product_id WHERE f.order_date >= CAST(date_trunc('quarter', DATE '2026-11-09') AS DATE) GROUP BY p.product_name, p.category ORDER BY net_revenue DESC LIMIT 10".into()),
          Check::None, &["sales_fact", "dim_product"]),
        s("W2.4", "W2", Ledger::Rewrite("DATE_TRUNC bound cast to DATE: DataFusion date_trunc on a date returns a nanosecond timestamp, which the Iceberg provider cannot push into a date column"),
          Some("SELECT SUM(quantity * unit_price * (1 - discount)) AS net_revenue_mtd, COUNT(DISTINCT customer_id) AS active_customers_mtd FROM ice.db.sales_fact WHERE order_date >= CAST(date_trunc('month', DATE '2026-11-09') AS DATE)".into()),
          Check::MtdTotals, &["sales_fact"]),
        s("W2.5", "W2", Ledger::Rewrite("DATEADD rewritten as timestamp minus INTERVAL (SF04)"),
          Some("SELECT event_type, COUNT(*) AS events, COUNT(DISTINCT session_id) AS sessions FROM ice.db.clickstream_events WHERE event_ts >= TIMESTAMP '2026-11-09 12:00:00' - INTERVAL '7' DAY GROUP BY event_type ORDER BY events DESC".into()),
          Check::Funnel, &["clickstream_events"]),
        // W3 Ad-hoc analyst analysis
        s("W3.1", "W3", Ledger::Rewrite("DATEADD rewritten as date minus INTERVAL (SF04)"),
          Some("SELECT p.category, c.segment, SUM(f.quantity * f.unit_price * (1 - f.discount)) AS net_revenue, COUNT(*) AS order_count FROM ice.db.sales_fact AS f JOIN ice.db.dim_product AS p ON p.product_id = f.product_id JOIN ice.db.dim_customer AS c ON c.customer_id = f.customer_id WHERE f.order_date >= DATE '2026-11-09' - INTERVAL '6' MONTH GROUP BY p.category, c.segment ORDER BY net_revenue DESC".into()),
          Check::W31Full, &["sales_fact", "dim_product", "dim_customer"]),
        s("W3.2", "W3", Ledger::Rewrite("DATE_TRUNC bound cast to DATE (DataFusion date_trunc on a date returns a nanosecond timestamp, which the Iceberg provider cannot push into a date column); window ORDER BY repeats the aggregate expression because DataFusion does not resolve the aggregate alias inside a window"),
          Some("SELECT customer_id, region, net_revenue, rnk FROM (SELECT customer_id, region, SUM(quantity * unit_price * (1 - discount)) AS net_revenue, RANK() OVER (PARTITION BY region ORDER BY SUM(quantity * unit_price * (1 - discount)) DESC) AS rnk FROM ice.db.sales_fact WHERE order_date >= CAST(date_trunc('year', DATE '2026-11-09') AS DATE) GROUP BY customer_id, region) AS ranked WHERE rnk <= 3 ORDER BY region, rnk".into()),
          Check::None, &["sales_fact"]),
        s("W3.3", "W3", Ledger::Rewrite("DATEADD rewritten as date minus INTERVAL (SF04)"),
          Some("SELECT region, COUNT(*) AS orders_checked, SUM(CASE WHEN discount > 0.4 THEN 1 ELSE 0 END) AS deep_discount_orders, AVG(discount) AS avg_discount FROM ice.db.sales_fact WHERE order_date >= DATE '2026-11-09' - INTERVAL '3' MONTH AND quantity * unit_price > 100 GROUP BY region ORDER BY deep_discount_orders DESC".into()),
          Check::None, &["sales_fact"]),
        s("W3.4", "W3", Ledger::Rewrite("grouped month cast to DATE (DataFusion date_trunc returns a timestamp)"),
          Some("WITH monthly AS (SELECT CAST(date_trunc('month', f.order_date) AS DATE) AS month, SUM(f.quantity * f.unit_price * (1 - f.discount)) AS net_revenue FROM ice.db.sales_fact AS f JOIN ice.db.dim_product AS p ON p.product_id = f.product_id WHERE p.category = 'Software' GROUP BY month) SELECT month, net_revenue, LAG(net_revenue) OVER (ORDER BY month) AS prior_month, net_revenue - LAG(net_revenue) OVER (ORDER BY month) AS change FROM monthly ORDER BY month".into()),
          Check::None, &["sales_fact", "dim_product"]),
        // W4 Embedded analytics (customer 42 stands in for :customer_id)
        s("W4.1", "W4", Ledger::Rewrite("DATE_TRUNC bound cast to DATE: DataFusion date_trunc on a date returns a nanosecond timestamp, which the Iceberg provider cannot push into a date column"),
          Some(format!("SELECT SUM(quantity * unit_price * (1 - discount)) AS net_revenue_mtd, COUNT(*) AS orders_mtd, COUNT(DISTINCT product_id) AS products_mtd FROM ice.db.sales_fact WHERE customer_id = {cust} AND order_date >= CAST(date_trunc('month', DATE '2026-11-09') AS DATE)")),
          Check::Cust42Mtd, &["sales_fact"]),
        s("W4.2", "W4", Ledger::Rewrite("DATEADD rewritten as date minus INTERVAL (SF04)"),
          Some(format!("SELECT p.product_name, SUM(f.quantity * f.unit_price * (1 - f.discount)) AS net_revenue FROM ice.db.sales_fact AS f JOIN ice.db.dim_product AS p ON p.product_id = f.product_id WHERE f.customer_id = {cust} AND f.order_date >= DATE '2026-11-09' - INTERVAL '90' DAY GROUP BY p.product_name ORDER BY net_revenue DESC LIMIT 10")),
          Check::None, &["sales_fact", "dim_product"]),
        s("W4.3", "W4", Ledger::Rewrite("DATEADD rewritten as date minus INTERVAL (SF04)"),
          Some(format!("SELECT CAST(date_trunc('month', order_date) AS DATE) AS month, SUM(quantity * unit_price * (1 - discount)) AS net_revenue, COUNT(*) AS order_count FROM ice.db.sales_fact WHERE customer_id = {cust} AND order_date >= DATE '2026-11-09' - INTERVAL '12' MONTH GROUP BY month ORDER BY month")),
          Check::None, &["sales_fact"]),
        s("W4.4", "W4", Ledger::Rewrite("DATEADD rewritten as timestamp minus INTERVAL (SF04)"),
          Some(format!("SELECT event_type, COUNT(*) AS events FROM ice.db.clickstream_events WHERE customer_id = {cust} AND event_ts >= TIMESTAMP '2026-11-09 12:00:00' - INTERVAL '30' DAY GROUP BY event_type ORDER BY events DESC")),
          Check::None, &["clickstream_events"]),
        // W5 ML feature builds
        s("W5.1", "W5", Ledger::Rewrite("TEMP TABLE wrapper dropped, the SELECT body runs; DATEADD as date minus INTERVAL (SF04); DATEDIFF(day) as the epoch form (docs/COMPATIBILITY.md SF05, rewrite W10)"),
          Some(FEATURES_BASE.into()), Check::DistinctCustomers, &["sales_fact"]),
        s("W5.2", "W5", Ledger::Rewrite("TEMP TABLE wrapper dropped; features_base inlined as a subquery"),
          Some(FEATURES_RANKED.into()), Check::DistinctCustomers, &["sales_fact", "dim_customer"]),
        s("W5.3", "W5", Ledger::Rewrite("TEMP TABLE wrapper dropped, the SELECT body runs; DATEADD as timestamp minus INTERVAL (SF04)"),
          Some(FEATURES_TREND.into()), Check::None, &["clickstream_events"]),
        s("W5.4s", "W5", Ledger::Rewrite("The MERGE source SELECT, run on its own: features_ranked LEFT JOIN features_trend, both inlined"),
          Some(format!("SELECT r.customer_id, r.recency_days, r.orders_12m, r.spend_12m, r.avg_order_value, r.spend_percentile_in_segment, r.spend_decile, t.events_this_month, t.activity_ratio FROM ({FEATURES_RANKED}) AS r LEFT JOIN ({FEATURES_TREND}) AS t ON t.customer_id = r.customer_id")),
          Check::DistinctCustomers, &["sales_fact", "dim_customer", "clickstream_events"]),
        s("W5.4", "W5", Ledger::Rewrite("MERGE runs through the storage writer (issue #18); its source is the W5.4s SELECT inlined"),
          Some(format!("MERGE INTO ice.db.features_customer AS f USING (SELECT r.customer_id, r.recency_days, r.orders_12m, r.spend_12m, r.avg_order_value, r.spend_percentile_in_segment, r.spend_decile, t.events_this_month, t.activity_ratio FROM ({FEATURES_RANKED}) AS r LEFT JOIN ({FEATURES_TREND}) AS t ON t.customer_id = r.customer_id) AS s ON f.customer_id = s.customer_id WHEN MATCHED THEN UPDATE SET f.recency_days = s.recency_days, f.orders_12m = s.orders_12m, f.spend_12m = s.spend_12m, f.avg_order_value = s.avg_order_value, f.spend_percentile_in_segment = s.spend_percentile_in_segment, f.spend_decile = s.spend_decile, f.events_this_month = s.events_this_month, f.activity_ratio = s.activity_ratio WHEN NOT MATCHED THEN INSERT (customer_id, recency_days, orders_12m, spend_12m, avg_order_value, spend_percentile_in_segment, spend_decile, events_this_month, activity_ratio) VALUES (s.customer_id, s.recency_days, s.orders_12m, s.spend_12m, s.avg_order_value, s.spend_percentile_in_segment, s.spend_decile, s.events_this_month, s.activity_ratio)")),
          Check::MergeFeatures, &["features_customer", "sales_fact", "dim_customer", "clickstream_events"]),
        s("W5.5", "W5", Ledger::Rewrite("UPDATE with FROM runs through the storage writer (issue #18); DATEDIFF(day) as the epoch form (SF05), DATEADD as date minus INTERVAL (SF04), CURRENT_DATE anchored"),
          Some("UPDATE ice.db.features_customer AS f SET recency_days = CAST((EXTRACT(EPOCH FROM DATE '2026-11-09') - EXTRACT(EPOCH FROM last_order.last_date)) / 86400 AS BIGINT) FROM (SELECT customer_id, MAX(order_date) AS last_date FROM ice.db.sales_fact WHERE order_date >= DATE '2026-11-09' - INTERVAL '2' DAY GROUP BY customer_id) AS last_order WHERE f.customer_id = last_order.customer_id".into()),
          Check::UpdateFromRecency, &["features_customer", "sales_fact"]),
        // W6 Agent-driven queries
        s("W6.1", "W6", Ledger::Rewrite("INFORMATION_SCHEMA.TABLES cannot run: the engine session does not enable information_schema (crates/opensnow-engine session config). The equivalent metadata operation runs against the Iceberg catalog API: list tables, then a row count per table from its current snapshot, timed as one operation"),
          None, Check::None, &[]),
        s("W6.2", "W6", Ledger::Rewrite("INFORMATION_SCHEMA.COLUMNS cannot run, same cause as W6.1. The equivalent runs against the Iceberg catalog API: load sales_fact and read its schema fields in order, timed as one operation"),
          None, Check::None, &[]),
        s("W6.3", "W6", Ledger::Rewrite("SAMPLE (100 ROWS): the row-count SAMPLE form is not supported; rewritten as a deterministic first 100 by event_id. Percent SAMPLE runs as written per spike B3 SF08 but changes the meaning."),
          Some("SELECT * FROM ice.db.clickstream_events ORDER BY event_id LIMIT 100".into()), Check::None, &["clickstream_events"]),
        s("W6.4", "W6", Ledger::Rewrite("TRY_CAST(quantity AS NUMBER): NUMBER is a Snowflake type name; rewritten as DECIMAL(10,2). DATEADD as timestamp minus INTERVAL (SF04)"),
          Some("SELECT region, COUNT(*) AS rows_seen, SUM(CASE WHEN TRY_CAST(quantity AS DECIMAL(10,2)) IS NULL THEN 1 ELSE 0 END) AS bad_quantities FROM ice.db.sales_fact WHERE order_date >= DATE '2026-11-09' - INTERVAL '7' DAY GROUP BY region".into()),
          Check::None, &["sales_fact"]),
        s("W6.5", "W6", Ledger::Rewrite("DATE_TRUNC bound cast to DATE: DataFusion date_trunc on a date returns a nanosecond timestamp, which the Iceberg provider cannot push into a date column"),
          Some("SELECT SUM(quantity * unit_price * (1 - discount)) AS net_revenue FROM ice.db.sales_fact WHERE order_date >= CAST(date_trunc('month', DATE '2026-11-09') AS DATE) AND region = 'East'".into()),
          Check::MtdEast, &["sales_fact"]),
        s("W6.6", "W6", Ledger::Rewrite("DATE_TRUNC bound cast to DATE: DataFusion date_trunc on a date returns a nanosecond timestamp, which the Iceberg provider cannot push into a date column"),
          Some("SELECT product_id, SUM(quantity * unit_price * (1 - discount)) AS net_revenue FROM ice.db.sales_fact WHERE order_date >= CAST(date_trunc('month', DATE '2026-11-09') AS DATE) AND region = 'East' GROUP BY product_id ORDER BY net_revenue DESC LIMIT 20".into()),
          Check::None, &["sales_fact"]),
        s("W6.7", "W6", Ledger::Rewrite("DATEADD rewritten as date minus INTERVAL (SF04)"),
          Some("SELECT COUNT(*) AS fact_rows_without_customer FROM ice.db.sales_fact AS f LEFT JOIN ice.db.dim_customer AS c ON c.customer_id = f.customer_id WHERE c.customer_id IS NULL AND f.order_date >= DATE '2026-11-09' - INTERVAL '30' DAY".into()),
          Check::Zero, &["sales_fact", "dim_customer"]),
    ]
}

// --- Result comparison ---

fn cells(batches: &[RecordBatch]) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    for batch in batches {
        for row in 0..batch.num_rows() {
            out.push(batch.columns().iter().map(|c| {
                if c.is_null(row) { "NULL".to_string() } else { array_value_to_string(c.as_ref(), row).unwrap_or_else(|_| "?".into()) }
            }).collect());
        }
    }
    out
}

fn num(cell: &str) -> f64 { cell.parse().unwrap_or(f64::NAN) }

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= (0.51f64).max(b.abs() * 1e-6)
}

pub fn run_check(check: Check, batches: &[RecordBatch], exp: &Expected) -> Result<(), String> {
    let rows = cells(batches);
    match check {
        Check::None => Ok(()),
        Check::MtdEast => {
            if rows.len() != 1 { return Err("MTD East: not one row".into()); }
            if !close(num(&rows[0][0]), exp.mtd_east_net) { return Err(format!("MTD East net {} != {}", rows[0][0], exp.mtd_east_net)); }
            Ok(())
        }
        Check::GlobalTotals => {
            if rows.len() != 1 { return Err(format!("global totals: {} rows", rows.len())); }
            if num(&rows[0][0]) as i64 != exp.sales_count { return Err(format!("count {} != {}", rows[0][0], exp.sales_count)); }
            if !close(num(&rows[0][1]), exp.sales_net) { return Err(format!("net {} != {}", rows[0][1], exp.sales_net)); }
            Ok(())
        }
        Check::DedupedCount => {
            let n: usize = batches.iter().map(|b| b.num_rows()).sum();
            if n as i64 != exp.w11_deduped { return Err(format!("deduped rows {n} != {}", exp.w11_deduped)); }
            Ok(())
        }
        Check::DailyAggRows => {
            let n: usize = batches.iter().map(|b| b.num_rows()).sum();
            if n != exp.w13_pairs.len() { return Err(format!("daily agg rows {n} != {}", exp.w13_pairs.len())); }
            Ok(())
        }
        Check::Zero => {
            if rows.len() != 1 || num(&rows[0][0]) != 0.0 { return Err(format!("expected a single 0, got {rows:?}")); }
            Ok(())
        }
        Check::W21Full | Check::W21West => {
            let mut want: Vec<((String, i64), (i64, f64))> = exp.w21.iter()
                .filter(|((region, _), _)| check == Check::W21Full || region == "West")
                .map(|(k, v)| (k.clone(), *v)).collect();
            want.sort_by(|a, b| a.0.cmp(&b.0));
            if rows.len() != want.len() { return Err(format!("W21 rows {} != {}", rows.len(), want.len())); }
            let mut got: Vec<(String, String, f64, f64)> = rows.iter()
                .map(|r| (r[0].clone(), r[1].clone(), num(&r[2]), num(&r[3]))).collect();
            got.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
            for (((region, mk), (count, net)), g) in want.iter().zip(got.iter()) {
                let (y, m, _) = civil_from_days({
                    // first day of the month for this key
                    let (yy, mm) = (mk / 12, mk % 12 + 1);
                    days_from_civil(yy, mm, 1)
                });
                let want_month = format!("{y:04}-{m:02}-01");
                if g.0 != *region || g.1 != want_month || g.3 as i64 != *count || !close(g.2, *net) {
                    return Err(format!("W21 mismatch: got {g:?}, want ({region}, {want_month}, {net}, {count})"));
                }
                let _ = (y, m);
            }
            Ok(())
        }
        Check::W31Full => {
            if rows.len() != exp.w31.len() { return Err(format!("W31 rows {} != {}", rows.len(), exp.w31.len())); }
            for r in &rows {
                let key = (r[0].clone(), r[1].clone());
                let (count, net) = exp.w31.get(&key).ok_or_else(|| format!("W31 unexpected group {key:?}"))?;
                if num(&r[3]) as i64 != *count || !close(num(&r[2]), *net) {
                    return Err(format!("W31 mismatch at {key:?}: got ({}, {}), want ({count}, {net})", r[2], r[3]));
                }
            }
            Ok(())
        }
        Check::MtdTotals => {
            if rows.len() != 1 { return Err("MTD: not one row".into()); }
            if !close(num(&rows[0][0]), exp.mtd_net) { return Err(format!("MTD net {} != {}", rows[0][0], exp.mtd_net)); }
            if num(&rows[0][1]) as i64 != exp.mtd_customers.len() as i64 {
                return Err(format!("MTD customers {} != {}", rows[0][1], exp.mtd_customers.len()));
            }
            Ok(())
        }
        Check::Cust42Mtd => {
            if rows.len() != 1 { return Err("cust42: not one row".into()); }
            if !close(num(&rows[0][0]), exp.cust42_mtd_net) { return Err(format!("cust42 net {} != {}", rows[0][0], exp.cust42_mtd_net)); }
            if num(&rows[0][1]) as i64 != exp.cust42_mtd_orders { return Err(format!("cust42 orders {} != {}", rows[0][1], exp.cust42_mtd_orders)); }
            if num(&rows[0][2]) as i64 != exp.cust42_mtd_products.len() as i64 {
                return Err(format!("cust42 products {} != {}", rows[0][2], exp.cust42_mtd_products.len()));
            }
            Ok(())
        }
        Check::Funnel => {
            if rows.len() != exp.w25.len() { return Err(format!("funnel rows {} != {}", rows.len(), exp.w25.len())); }
            for r in &rows {
                let (events, sessions) = exp.w25.get(&r[0]).ok_or_else(|| format!("funnel unexpected type {}", r[0]))?;
                if num(&r[1]) as i64 != *events || num(&r[2]) as i64 != sessions.len() as i64 {
                    return Err(format!("funnel mismatch at {}: got ({}, {}), want ({events}, {})", r[0], r[1], r[2], sessions.len()));
                }
            }
            Ok(())
        }
        Check::DistinctCustomers => {
            let n: usize = batches.iter().map(|b| b.num_rows()).sum();
            if n != exp.distinct_customers_sales.len() {
                return Err(format!("rows {n} != distinct customers {}", exp.distinct_customers_sales.len()));
            }
            Ok(())
        }
        Check::MergeSales | Check::MergeAgg | Check::MergeFeatures | Check::UpdateFromRecency => {
            Err("mutation checks read the target table, not result batches".into())
        }
    }
}

// --- Runner ---

pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Clone, Debug, Default)]
pub struct StmtResult {
    pub id: String,
    pub workload: String,
    pub ledger: String,
    pub status: String, // timed | could_not_run | failed
    pub median_seconds: Option<f64>,
    pub runs_seconds: Vec<f64>,
    pub rows: Option<usize>,
    pub check: String, // pass | fail: ... | none
}

#[derive(Clone, Debug, Default)]
pub struct Report {
    pub divisor: i64,
    pub gen_seconds: f64,
    pub load_seconds: f64,
    pub cold_start_seconds: Vec<f64>,
    pub statements: Vec<StmtResult>,
    pub global_check: String,
    pub write_check: String,
    pub burst: String,
    /// table name -> (data files, parquet bytes)
    pub files: Vec<(String, usize, u64)>,
    pub total_parquet_bytes: u64,
}

fn ledger_text(l: &Ledger) -> String {
    match l {
        Ledger::AsWritten => "ran as written".to_string(),
        Ledger::Rewrite(why) => format!("ran with rewrite: {why}"),
        Ledger::CouldNotRun(why) => format!("could not run: {why}"),
    }
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

pub struct Harness {
    pub storage: StorageCatalog,
    pub service: EngineService,
    pub dir: tempfile::TempDir,
}

/// Full evaluation run at one scale divisor. Writes nothing to disk
/// outside a temp warehouse that the caller keeps alive via the report's
/// harness handle.
pub async fn run(divisor: i64) -> Result<(Report, Harness, Dataset), BoxError> {
    // Cold start, measured on fresh catalog connections over a scratch
    // warehouse: connect, register the provider, first query.
    let mut cold = Vec::new();
    for _ in 0..3 {
        let t = Instant::now();
        let d = tempfile::tempdir()?;
        let cfg = CatalogConfig::sqlite("cold", d.path().join("c.db"), d.path().join("w"))?;
        let s = cfg.connect().await?;
        let ctx = SessionContext::new();
        ctx.register_catalog("ice", Arc::new(IcebergCatalogProvider::try_new(s.catalog.clone()).await?));
        ctx.sql("SELECT 1").await?.collect().await?;
        cold.push(t.elapsed().as_secs_f64());
    }

    let dir = tempfile::tempdir()?;
    let cfg = CatalogConfig::sqlite("gate1", dir.path().join("catalog.db"), dir.path().join("warehouse"))?;
    let storage = cfg.connect().await?;
    storage.create_namespace(&NamespaceIdent::new("db".into())).await?;
    let gen_start = Instant::now();
    let data = generate(Scale::new(divisor));
    let gen_seconds = gen_start.elapsed().as_secs_f64();
    let load_start = Instant::now();
    for ddl in [
        "CREATE TABLE ice.db.sales_fact (order_id BIGINT NOT NULL, order_date DATE NOT NULL, customer_id BIGINT NOT NULL, product_id BIGINT NOT NULL, region VARCHAR NOT NULL, quantity BIGINT NOT NULL, unit_price DOUBLE NOT NULL, discount DOUBLE NOT NULL)",
        "CREATE TABLE ice.db.clickstream_events (event_id BIGINT NOT NULL, session_id BIGINT NOT NULL, customer_id BIGINT NOT NULL, event_type VARCHAR NOT NULL, page VARCHAR NOT NULL, event_ts TIMESTAMP(6) NOT NULL)",
        "CREATE TABLE ice.db.orders_raw (order_id BIGINT NOT NULL, order_date DATE NOT NULL, customer_id BIGINT NOT NULL, product_id BIGINT NOT NULL, region VARCHAR NOT NULL, quantity BIGINT NOT NULL, unit_price DOUBLE NOT NULL, discount DOUBLE NOT NULL, loaded_at TIMESTAMP(6) NOT NULL)",
        "CREATE TABLE ice.db.dim_customer (customer_id BIGINT NOT NULL, customer_name VARCHAR NOT NULL, segment VARCHAR NOT NULL, country VARCHAR NOT NULL, signup_date DATE NOT NULL)",
        "CREATE TABLE ice.db.dim_product (product_id BIGINT NOT NULL, category VARCHAR NOT NULL, product_name VARCHAR NOT NULL, brand VARCHAR NOT NULL)",
        "CREATE TABLE ice.db.features_customer (customer_id BIGINT NOT NULL, recency_days BIGINT, orders_12m BIGINT, spend_12m DOUBLE, avg_order_value DOUBLE, spend_percentile_in_segment DOUBLE, spend_decile BIGINT, events_this_month BIGINT, activity_ratio DOUBLE)",
        "CREATE TABLE ice.db.daily_revenue_agg (order_date DATE NOT NULL, region VARCHAR NOT NULL, net_revenue DOUBLE, order_count BIGINT, avg_daily_revenue DOUBLE)",
        "CREATE TABLE ice.db.write_check (order_id BIGINT NOT NULL, order_date DATE NOT NULL, customer_id BIGINT NOT NULL, product_id BIGINT NOT NULL, region VARCHAR NOT NULL, quantity BIGINT NOT NULL, unit_price DOUBLE NOT NULL, discount DOUBLE NOT NULL)",
    ] {
        storage.sql(ddl).await?;
    }
    let ns = NamespaceIdent::new("db".into());
    storage.replace_all(&TableIdent::new(ns.clone(), "sales_fact".into()), data.sales.clone()).await?;
    storage.replace_all(&TableIdent::new(ns.clone(), "clickstream_events".into()), data.clickstream.clone()).await?;
    storage.replace_all(&TableIdent::new(ns.clone(), "orders_raw".into()), data.orders_raw.clone()).await?;
    storage.replace_all(&TableIdent::new(ns.clone(), "dim_customer".into()), data.dim_customer.clone()).await?;
    storage.replace_all(&TableIdent::new(ns.clone(), "dim_product".into()), data.dim_product.clone()).await?;
    let load_seconds = load_start.elapsed().as_secs_f64();

    let mut service = EngineService::new();
    service.register_tenant("gate1", ResourceGroup {
        memory_bytes: 2 * 1024 * 1024 * 1024,
        target_partitions: 2,
        max_concurrent: 1,
        queue_capacity: 64,
        queue_timeout: Duration::from_secs(120),
        cost_cap_units: None,
    })?;
    service.context("gate1")?.register_catalog(
        "ice", Arc::new(IcebergCatalogProvider::try_new(storage.catalog.clone()).await?));
    service.register_tenant("burst", ResourceGroup {
        memory_bytes: 2 * 1024 * 1024 * 1024,
        target_partitions: 2,
        max_concurrent: 32,
        queue_capacity: 64,
        queue_timeout: Duration::from_secs(300),
        cost_cap_units: None,
    })?;
    service.context("burst")?.register_catalog(
        "ice", Arc::new(IcebergCatalogProvider::try_new(storage.catalog.clone()).await?));

    let mut report = Report {
        divisor, gen_seconds, load_seconds, cold_start_seconds: cold, ..Default::default()
    };

    // Global yardstick first: count and net revenue over the whole fact.
    let global = service.execute("gate1",
        "SELECT COUNT(*) AS n, SUM(quantity * unit_price * (1 - discount)) AS net FROM ice.db.sales_fact").await;
    report.global_check = match global {
        Ok(r) => match run_check(Check::GlobalTotals, &r.batches, &data.expected) {
            Ok(()) => "pass".to_string(),
            Err(e) => format!("fail: {e}"),
        },
        Err(e) => format!("fail: execute: {e}"),
    };

    // W6.1 metadata equivalent: catalog list plus per-table row counts.
    let w61_sql_tables = service.execute("gate1",
        "SELECT table_name FROM information_schema.tables WHERE table_schema = 'db' ORDER BY table_name").await;

    for stmt in statements(data.probe_customer) {
        let mut sr = StmtResult {
            id: stmt.id.to_string(),
            workload: stmt.workload.to_string(),
            ledger: ledger_text(&stmt.ledger),
            status: "timed".to_string(),
            check: "none".to_string(),
            ..Default::default()
        };
        if matches!(stmt.ledger, Ledger::CouldNotRun(_)) {
            sr.status = "could_not_run".to_string();
            report.statements.push(sr);
            continue;
        }
        if stmt.id == "W6.1" {
            // Time the metadata equivalent: information_schema names (if
            // the engine serves it) plus catalog row counts and sizes.
            let t = Instant::now();
            let mut runs = Vec::new();
            let mut ok = true;
            let mut table_count = 0usize;
            for _ in 0..3 {
                let start = Instant::now();
                let tables = storage.catalog.list_tables(&ns).await;
                match tables {
                    Ok(idents) => {
                        table_count = idents.len();
                        for ident in &idents {
                            if let Ok(snap) = storage.current_snapshot_id(ident).await {
                                if storage.scan_snapshot(ident, snap).await.is_err() { ok = false; }
                            }
                        }
                    }
                    Err(_) => ok = false,
                }
                runs.push(start.elapsed().as_secs_f64());
            }
            let _ = t;
            sr.median_seconds = Some(median(runs.clone()));
            sr.runs_seconds = runs;
            sr.rows = Some(table_count);
            sr.check = if ok { "pass".into() } else { "fail: catalog metadata op".into() };
            let _ = &w61_sql_tables;
            report.statements.push(sr);
            continue;
        }
        if stmt.id == "W6.2" {
            let ident = TableIdent::new(ns.clone(), "sales_fact".into());
            let want = ["order_id", "order_date", "customer_id", "product_id", "region", "quantity", "unit_price", "discount"];
            let mut runs = Vec::new();
            let mut names: Vec<String> = Vec::new();
            for _ in 0..3 {
                let start = Instant::now();
                match storage.catalog.load_table(&ident).await {
                    Ok(table) => {
                        names = table.metadata().current_schema().as_struct().fields().iter().map(|f| f.name.clone()).collect();
                    }
                    Err(_) => { names = Vec::new(); }
                }
                runs.push(start.elapsed().as_secs_f64());
            }
            sr.median_seconds = Some(median(runs.clone()));
            sr.runs_seconds = runs;
            sr.rows = Some(names.len());
            sr.check = if names.iter().map(|s| s.as_str()).collect::<Vec<_>>() == want { "pass".into() } else { format!("fail: schema fields {names:?}") };
            report.statements.push(sr);
            continue;
        }
        if matches!(stmt.id, "W1.2" | "W1.3b" | "W5.4" | "W5.5") {
            // Mutations run through the storage writer, not the engine
            // session, and are timed the same way: one warm-up, three runs.
            let sql = stmt.sql.as_ref().expect("mutation statement has sql");
            let mut runs = Vec::new();
            let mut failure = None;
            for i in 0..4 {
                let start = Instant::now();
                match storage.execute(sql).await {
                    Ok(_) => {
                        if i > 0 {
                            runs.push(start.elapsed().as_secs_f64());
                        }
                    }
                    Err(e) => {
                        failure = Some(format!("{e}"));
                        break;
                    }
                }
            }
            if let Some(e) = failure {
                sr.status = "failed".to_string();
                sr.check = format!("fail: execute: {e}");
                report.statements.push(sr);
                continue;
            }
            sr.median_seconds = Some(median(runs.clone()));
            sr.runs_seconds = runs;
            sr.rows = Some(0);
            sr.check = match mutation_check(stmt.id, &storage, &data).await {
                Ok(()) => "pass".to_string(),
                Err(e) => format!("fail: {e}"),
            };
            if stmt.id == "W1.2" {
                // The generated orders_raw and sales_fact sets are
                // independent fixtures, and the read workloads' yardsticks
                // describe the original fact table. The merge has run and
                // been checked; restore the fixture before W1.3 reads it.
                if let Err(e) = storage.replace_all(
                    &TableIdent::new(ns.clone(), "sales_fact".into()), data.sales.clone(),
                ).await {
                    sr.check = format!("fail: restore sales_fact after W1.2: {e}");
                }
            }
            report.statements.push(sr);
            continue;
        }
        let sql = stmt.sql.as_ref().expect("runnable statement has sql");
        // Warm-up, untimed.
        if let Err(e) = service.execute("gate1", sql).await {
            sr.status = "failed".to_string();
            sr.check = format!("fail: warm-up execute: {e}");
            report.statements.push(sr);
            continue;
        }
        let mut runs = Vec::new();
        let mut last = None;
        let mut failure = None;
        for _ in 0..3 {
            match service.execute("gate1", sql).await {
                Ok(r) => { runs.push(r.outcome.elapsed.as_secs_f64()); last = Some(r); }
                Err(e) => { failure = Some(format!("{e}")); break; }
            }
        }
        if let Some(e) = failure {
            sr.status = "failed".to_string();
            sr.check = format!("fail: execute: {e}");
            report.statements.push(sr);
            continue;
        }
        let last = last.unwrap();
        sr.rows = Some(last.batches.iter().map(|b| b.num_rows()).sum());
        sr.median_seconds = Some(median(runs.clone()));
        sr.runs_seconds = runs;
        sr.check = match run_check(stmt.check, &last.batches, &data.expected) {
            Ok(()) => if stmt.check == Check::None { "none".to_string() } else { "pass".to_string() },
            Err(e) => format!("fail: {e}"),
        };
        report.statements.push(sr);
    }

    // Write path check: land W1.3 rows in daily_revenue_agg, land W5.4s
    // rows in features_customer, then a copy-on-write DELETE on a scratch
    // table filled with the first sales batches.
    report.write_check = write_path_check(&storage, &service, &data, &ns).await;

    // W4 burst: 30 concurrent home-screen queries, distinct customers.
    report.burst = burst_check(&service).await;

    // File layout from the warehouse directory.
    let warehouse = dir.path().join("warehouse");
    let mut files: HashMap<String, (usize, u64)> = HashMap::new();
    fn walk(p: &std::path::Path, files: &mut HashMap<String, (usize, u64)>) {
        if let Ok(rd) = std::fs::read_dir(p) {
            for e in rd.flatten() {
                let path = e.path();
                if path.is_dir() { walk(&path, files); }
                else if path.extension().is_some_and(|x| x == "parquet") {
                    let table = path.ancestors().nth(2).map(|a| a.file_name().unwrap_or_default().to_string_lossy().into_owned()).unwrap_or_default();
                    let entry = files.entry(table).or_default();
                    entry.0 += 1;
                    entry.1 += e.metadata().map(|m| m.len()).unwrap_or(0);
                }
            }
        }
    }
    walk(&warehouse, &mut files);
    report.total_parquet_bytes = files.values().map(|v| v.1).sum();
    let mut fv: Vec<_> = files.into_iter().map(|(k, (c, b))| (k, c, b)).collect();
    fv.sort();
    report.files = fv;

    let harness = Harness { storage, service, dir };
    Ok((report, harness, data))
}

// --- Mutation checks (issue #18) ---
//
// Expected post-mutation rows are built from the generated batches with
// plain Rust loops, the same yardstick style as run_check: nothing here
// reads a result the SQL under test produced.

type SalesRow = (i64, i32, i64, i64, String, i64, f64, f64);

fn sales_rows(batches: &[RecordBatch]) -> Vec<SalesRow> {
    use arrow::array::{Array, Date32Array, Float64Array, Int64Array, StringArray};
    let mut out = Vec::new();
    for batch in batches {
        let ids = batch.column(0).as_any().downcast_ref::<Int64Array>().unwrap();
        let dates = batch.column(1).as_any().downcast_ref::<Date32Array>().unwrap();
        let custs = batch.column(2).as_any().downcast_ref::<Int64Array>().unwrap();
        let prods = batch.column(3).as_any().downcast_ref::<Int64Array>().unwrap();
        let regs = batch.column(4).as_any().downcast_ref::<StringArray>().unwrap();
        let qtys = batch.column(5).as_any().downcast_ref::<Int64Array>().unwrap();
        let prices = batch.column(6).as_any().downcast_ref::<Float64Array>().unwrap();
        let discs = batch.column(7).as_any().downcast_ref::<Float64Array>().unwrap();
        for i in 0..batch.num_rows() {
            out.push((
                ids.value(i), dates.value(i), custs.value(i), prods.value(i),
                regs.value(i).to_string(), qtys.value(i), prices.value(i), discs.value(i),
            ));
        }
        let _ = batch;
    }
    out
}

fn feature_rows(batches: &[RecordBatch]) -> Vec<(i64, Option<i64>, Option<i64>, Option<f64>, Option<f64>)> {
    use arrow::array::{Array, Float64Array, Int64Array};
    let mut out = Vec::new();
    for batch in batches {
        let custs = batch.column(0).as_any().downcast_ref::<Int64Array>().unwrap();
        let recency = batch.column(1).as_any().downcast_ref::<Int64Array>().unwrap();
        let orders = batch.column(2).as_any().downcast_ref::<Int64Array>().unwrap();
        let spend = batch.column(3).as_any().downcast_ref::<Float64Array>().unwrap();
        let avg = batch.column(4).as_any().downcast_ref::<Float64Array>().unwrap();
        for i in 0..batch.num_rows() {
            out.push((
                custs.value(i),
                (!recency.is_null(i)).then(|| recency.value(i)),
                (!orders.is_null(i)).then(|| orders.value(i)),
                (!spend.is_null(i)).then(|| spend.value(i)),
                (!avg.is_null(i)).then(|| avg.value(i)),
            ));
        }
    }
    out
}

async fn mutation_check(id: &str, storage: &StorageCatalog, data: &Dataset) -> Result<(), String> {
    let anchor = anchor_day();
    match id {
        "W1.2" => {
            let mut merged: HashMap<i64, SalesRow> = HashMap::new();
            for row in sales_rows(&data.sales) {
                merged.insert(row.0, row);
            }
            // Dedupe orders_raw loaded in the last day, latest load wins.
            let cutoff = (anchor * 86_400 + 43_200 - 86_400) * 1_000_000;
            let mut latest: HashMap<i64, (i64, SalesRow)> = HashMap::new();
            for batch in &data.orders_raw {
                let rows = sales_rows(&[RecordBatch::try_new(
                    sales_schema(),
                    batch.columns()[..8].to_vec(),
                ).unwrap()]);
                use arrow::array::{Array, TimestampMicrosecondArray};
                let loads = batch.column(8).as_any().downcast_ref::<TimestampMicrosecondArray>().unwrap();
                for (i, row) in rows.into_iter().enumerate() {
                    let loaded = loads.value(i);
                    if loaded >= cutoff {
                        let entry = latest.entry(row.0).or_insert((i64::MIN, row.clone()));
                        if loaded > entry.0 {
                            *entry = (loaded, row);
                        }
                    }
                }
            }
            for (order_id, (_, row)) in latest {
                merged.insert(order_id, row);
            }
            let got_batches = storage.sql(
                "SELECT order_id, order_date, customer_id, product_id, region, quantity, unit_price, discount FROM ice.db.sales_fact",
            ).await.map_err(|e| e.to_string())?;
            let got: HashMap<i64, SalesRow> = sales_rows(&got_batches).into_iter().map(|r| (r.0, r)).collect();
            if got.len() != merged.len() {
                return Err(format!("merged sales_fact rows {} != {}", got.len(), merged.len()));
            }
            for (order_id, want) in &merged {
                if got.get(order_id) != Some(want) {
                    return Err(format!("merged sales_fact row {order_id} differs"));
                }
            }
            Ok(())
        }
        "W1.3b" => {
            let mut daily: HashMap<(i32, String), (i64, f64)> = HashMap::new();
            for row in sales_rows(&data.sales) {
                if (row.1 as i64) >= anchor - 35 {
                    let entry = daily.entry((row.1, row.4.clone())).or_default();
                    entry.0 += 1;
                    entry.1 += row.5 as f64 * row.6 * (1.0 - row.7);
                }
            }
            let mut monthly: HashMap<(i64, String), (f64, i64)> = HashMap::new();
            for ((day, region), (_, net)) in &daily {
                let entry = monthly.entry((month_key(*day as i64), region.clone())).or_default();
                entry.0 += net;
                entry.1 += 1;
            }
            let got_batches = storage.sql(
                "SELECT order_date, region, net_revenue, order_count, avg_daily_revenue FROM ice.db.daily_revenue_agg",
            ).await.map_err(|e| e.to_string())?;
            use arrow::array::{Array, Date32Array, Float64Array, Int64Array, StringArray};
            let mut got: HashMap<(i32, String), (i64, f64, f64)> = HashMap::new();
            for batch in &got_batches {
                let dates = batch.column(0).as_any().downcast_ref::<Date32Array>().unwrap();
                let regs = batch.column(1).as_any().downcast_ref::<StringArray>().unwrap();
                let nets = batch.column(2).as_any().downcast_ref::<Float64Array>().unwrap();
                let counts = batch.column(3).as_any().downcast_ref::<Int64Array>().unwrap();
                let avgs = batch.column(4).as_any().downcast_ref::<Float64Array>().unwrap();
                for i in 0..batch.num_rows() {
                    got.insert(
                        (dates.value(i), regs.value(i).to_string()),
                        (counts.value(i), nets.value(i), avgs.value(i)),
                    );
                }
            }
            if got.len() != daily.len() {
                return Err(format!("daily_revenue_agg rows {} != {}", got.len(), daily.len()));
            }
            for ((day, region), (count, net)) in &daily {
                let (got_count, got_net, got_avg) = got.get(&(*day, region.clone()))
                    .ok_or_else(|| format!("daily_revenue_agg missing ({day}, {region})"))?;
                let (sum, days) = monthly[&(month_key(*day as i64), region.clone())];
                if got_count != count || !close(*got_net, *net) || !close(*got_avg, sum / days as f64) {
                    return Err(format!("daily_revenue_agg mismatch at ({day}, {region})"));
                }
            }
            Ok(())
        }
        "W5.4" => {
            let got_batches = storage.sql(
                "SELECT customer_id, recency_days, orders_12m, spend_12m, avg_order_value FROM ice.db.features_customer",
            ).await.map_err(|e| e.to_string())?;
            let got = feature_rows(&got_batches);
            if got.len() != data.expected.distinct_customers_sales.len() {
                return Err(format!(
                    "features_customer rows {} != distinct customers {}",
                    got.len(), data.expected.distinct_customers_sales.len()));
            }
            let got_ids: HashSet<i64> = got.iter().map(|r| r.0).collect();
            if got_ids != data.expected.distinct_customers_sales {
                return Err("features_customer customer set differs".into());
            }
            // Probe customer yardstick from the generated sales rows.
            let day_minus_12m = days_from_civil(2025, 11, 9);
            let (mut count, mut spend, mut max_day) = (0i64, 0.0f64, i64::MIN);
            for row in sales_rows(&data.sales) {
                if row.2 == data.probe_customer && (row.1 as i64) >= day_minus_12m {
                    count += 1;
                    spend += row.5 as f64 * row.6 * (1.0 - row.7);
                    max_day = max_day.max(row.1 as i64);
                }
            }
            let probe = got.iter().find(|r| r.0 == data.probe_customer)
                .ok_or("features_customer missing probe customer")?;
            if probe.1 != Some(anchor - max_day) || probe.2 != Some(count)
                || !close(probe.3.unwrap_or(f64::NAN), spend)
                || !close(probe.4.unwrap_or(f64::NAN), spend / count as f64) {
                return Err(format!("features_customer probe row differs: {probe:?}"));
            }
            Ok(())
        }
        "W5.5" => {
            let mut last2: HashMap<i64, i32> = HashMap::new();
            for row in sales_rows(&data.sales) {
                if (row.1 as i64) >= anchor - 2 {
                    let entry = last2.entry(row.2).or_insert(row.1);
                    if row.1 > *entry {
                        *entry = row.1;
                    }
                }
            }
            let got_batches = storage.sql(
                "SELECT customer_id, recency_days, orders_12m, spend_12m, avg_order_value FROM ice.db.features_customer",
            ).await.map_err(|e| e.to_string())?;
            let got = feature_rows(&got_batches);
            let mut seen = 0usize;
            for row in &got {
                if let Some(day) = last2.get(&row.0) {
                    seen += 1;
                    if row.1 != Some(anchor - *day as i64) {
                        return Err(format!(
                            "W5.5 recency for customer {} is {:?}, want {}",
                            row.0, row.1, anchor - *day as i64));
                    }
                }
            }
            if seen != last2.len() {
                return Err(format!("W5.5 refreshed {seen} customers, want {}", last2.len()));
            }
            Ok(())
        }
        other => Err(format!("no mutation check for {other}")),
    }
}

async fn write_path_check(storage: &StorageCatalog, service: &EngineService, data: &Dataset, ns: &NamespaceIdent) -> String {
    let mut notes = Vec::new();
    // Land the daily aggregate (W1.3's result) as one overwrite snapshot.
    let w13 = statements(data.probe_customer).into_iter().find(|s| s.id == "W1.3").unwrap();
    match service.execute("gate1", w13.sql.as_ref().unwrap()).await {
        Ok(r) => {
            let rows: usize = r.batches.iter().map(|b| b.num_rows()).sum();
            match storage.replace_all(&TableIdent::new(ns.clone(), "daily_revenue_agg".into()), r.batches).await {
                Ok(()) => notes.push(format!("daily_revenue_agg landed {rows} rows via replace_all")),
                Err(e) => return format!("fail: replace_all daily_revenue_agg: {e}"),
            }
        }
        Err(e) => return format!("fail: W1.3 rerun for write check: {e}"),
    }
    // Land the feature rows (W5.4s result) the same way.
    let w54 = statements(data.probe_customer).into_iter().find(|s| s.id == "W5.4s").unwrap();
    match service.execute("gate1", w54.sql.as_ref().unwrap()).await {
        Ok(r) => {
            let rows: usize = r.batches.iter().map(|b| b.num_rows()).sum();
            match storage.replace_all(&TableIdent::new(ns.clone(), "features_customer".into()), r.batches).await {
                Ok(()) => notes.push(format!("features_customer landed {rows} rows via replace_all")),
                Err(e) => return format!("fail: replace_all features_customer: {e}"),
            }
        }
        Err(e) => return format!("fail: W5.4s rerun for write check: {e}"),
    }
    // Copy-on-write DELETE through the storage writer.
    let first: Vec<RecordBatch> = data.sales.iter().take(2).cloned().collect();
    let scratch_rows: usize = first.iter().map(|b| b.num_rows()).sum();
    if let Err(e) = storage.replace_all(&TableIdent::new(ns.clone(), "write_check".into()), first).await {
        return format!("fail: replace_all write_check: {e}");
    }
    let east = (0..scratch_rows).filter(|k| k % 4 == 0).count();
    match storage.execute("DELETE FROM ice.db.write_check WHERE region = 'East'").await {
        Ok(_) => match storage.sql("SELECT COUNT(*) FROM ice.db.write_check").await {
            Ok(batches) => {
                let left = cells(&batches)[0][0].clone();
                if left.parse::<usize>() == Ok(scratch_rows - east) {
                    notes.push(format!("copy-on-write DELETE removed {east} East rows, {} remain", scratch_rows - east));
                } else {
                    return format!("fail: after DELETE count {left}, want {}", scratch_rows - east);
                }
            }
            Err(e) => return format!("fail: count after DELETE: {e}"),
        },
        Err(e) => return format!("fail: DELETE execute: {e}"),
    }
    format!("pass: {}", notes.join("; "))
}

async fn burst_check(service: &EngineService) -> String {
    let template = "SELECT SUM(quantity * unit_price * (1 - discount)) AS net_revenue_mtd, COUNT(*) AS orders_mtd, COUNT(DISTINCT product_id) AS products_mtd FROM ice.db.sales_fact WHERE customer_id = {} AND order_date >= CAST(date_trunc('month', DATE '2026-11-09') AS DATE)";
    let start = Instant::now();
    let sqls: Vec<String> = (0..30i64).map(|c| template.replace("{}", &(1000 + c).to_string())).collect();
    let mut futs = Vec::new();
    for sql in &sqls {
        futs.push(service.execute("burst", sql));
    }
    let results = futures::future::join_all(futs).await;
    let wall = start.elapsed().as_secs_f64();
    let ok = results.iter().filter(|r| r.is_ok()).count();
    let first_err = results.iter().find_map(|r| r.as_ref().err().map(|e| e.to_string())).unwrap_or_default();
    format!("{ok}/30 concurrent W4.1 queries completed, wall {wall:.2}s, first error: {first_err}")
}

/// Registers generated data as MemTables on a plain context. Used by the
/// crate's own tests to probe dialect questions without Iceberg.
pub async fn probe_ctx(data: &Dataset) -> Result<SessionContext, BoxError> {
    let ctx = SessionContext::new();
    ctx.register_table("sales_fact", Arc::new(MemTable::try_new(sales_schema(), vec![data.sales.clone()])?))?;
    ctx.register_table("clickstream_events", Arc::new(MemTable::try_new(click_schema(), vec![data.clickstream.clone()])?))?;
    Ok(ctx)
}
