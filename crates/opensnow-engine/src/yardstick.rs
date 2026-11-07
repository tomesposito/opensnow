//! Plain Rust standing form of the B1 DuckDB reference arithmetic.
use std::collections::BTreeMap;
use datafusion::{arrow::{array::{Array, Date32Array}, compute::cast, datatypes::DataType, util::display::array_value_to_string}, error::Result, prelude::*};

pub async fn check(ctx: &SessionContext) -> Result<()> {
    let mut west = (0u64, 0.0);
    let mut groups = BTreeMap::<(String, usize), (u64, f64)>::new();
    let mut categories = BTreeMap::<String, (u64, f64)>::new();
    let mut net = 0.0;
    for i in 0..24 {
        let region = if i % 2 == 0 { "West" } else { "East" };
        let category = if i % 2 == 0 { "hardware" } else { "software" };
        let revenue = (i % 4 + 1) as f64 * (i + 10) as f64 * (1.0 - if i % 2 == 0 { 0.25 } else { 0.0 });
        net += revenue;
        if region == "West" { west.0 += 1; west.1 += revenue; }
        let group = groups.entry((region.into(), i % 3)).or_default();
        group.0 += 1; group.1 += revenue;
        let group = categories.entry(category.into()).or_default();
        group.0 += 1; group.1 += revenue;
    }
    let queries: Vec<_> = include_str!("queries.sql").lines().filter(|s| !s.starts_with("--") && !s.trim().is_empty()).collect();
    let q1 = ctx.sql(queries[0]).await?.collect().await?;
    assert_eq!(q1.iter().map(|b| b.num_rows()).sum::<usize>(), 1);
    number(q1[0].column(0).as_ref(), 0, west.0 as f64);
    number(q1[0].column(1).as_ref(), 0, west.1);
    let q2 = ctx.sql(queries[1]).await?.collect().await?;
    let mut expected = groups.iter();
    for batch in q2 {
        let dates = cast(batch.column(1), &DataType::Date32)?;
        let dates = dates.as_any().downcast_ref::<Date32Array>().unwrap();
        for row in 0..batch.num_rows() {
            let ((region, month), (count, revenue)) = expected.next().expect("extra Q2 row");
            assert_eq!(array_value_to_string(batch.column(0).as_ref(), row)?, *region);
            assert_eq!(dates.value(row), [19723, 19754, 19783][*month]);
            number(batch.column(2).as_ref(), row, *revenue);
            number(batch.column(3).as_ref(), row, *count as f64);
        }
    }
    assert!(expected.next().is_none(), "missing Q2 row");
    let mut categories: Vec<_> = categories.into_iter().collect();
    categories.sort_by(|a,b| b.1.1.total_cmp(&a.1.1));
    let mut expected = categories.iter();
    for batch in ctx.sql(queries[2]).await?.collect().await? {
        for row in 0..batch.num_rows() {
            let (category, (count, revenue)) = expected.next().expect("extra Q3 row");
            assert_eq!(array_value_to_string(batch.column(0).as_ref(), row)?, *category);
            number(batch.column(1).as_ref(), row, *count as f64);
            number(batch.column(2).as_ref(), row, *revenue);
        }
    }
    assert!(expected.next().is_none(), "missing Q3 row");
    let batches = ctx.sql("SELECT sum(net_price(CAST(quantity AS BIGINT), CAST(unit_price AS DOUBLE), CAST(discount AS DOUBLE))) FROM sales").await?.collect().await?;
    assert_eq!(batches.iter().map(|b| b.num_rows()).sum::<usize>(), 1);
    number(batches[0].column(0).as_ref(), 0, net);
    println!("PASS yardstick value diffs: Q1, Q2, Q3, net_price");
    Ok(())
}
fn number(array: &dyn Array, row: usize, expected: f64) {
    assert!(!array.is_null(row));
    let actual: f64 = array_value_to_string(array, row).unwrap().parse().unwrap();
    assert!((actual - expected).abs() < 1e-9, "yardstick: {actual} != {expected}");
}
