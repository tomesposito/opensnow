// Iceberg + DataFusion spike for opensnow Archetype B.
// Each step logs OK/FAIL and the program continues where it can.
use std::collections::HashMap;
use std::sync::Arc;

use arrow::array::{Float64Array, Int64Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema as ArrowSchema};
use arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use iceberg::{Catalog, CatalogBuilder, NamespaceIdent};
use iceberg_catalog_sql::SqlCatalogBuilder;
use iceberg_datafusion::IcebergCatalogProvider;

fn report(step: &str, r: &Result<String, String>) {
    match r {
        Ok(msg) => println!("OK   {step}: {msg}"),
        Err(e) => println!("FAIL {step}: {e}"),
    }
}

fn batch(start: i64, n: i64, city: &str, amt: f64) -> RecordBatch {
    let schema = Arc::new(ArrowSchema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("city", DataType::Utf8, true),
        Field::new("amount", DataType::Float64, true),
    ]));
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Int64Array::from_iter(start..start + n)),
            Arc::new(StringArray::from_iter(std::iter::repeat_n(
                Some(city),
                n as usize,
            ))),
            Arc::new(Float64Array::from_iter(std::iter::repeat_n(
                Some(amt),
                n as usize,
            ))),
        ],
    )
    .unwrap()
}

async fn sql_count(ctx: &SessionContext, sql: &str) -> Result<i64, String> {
    let df = ctx.sql(sql).await.map_err(|e| e.to_string())?;
    let batches = df.collect().await.map_err(|e| e.to_string())?;
    let b = batches.first().ok_or("no batches")?;
    let col = b
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .ok_or("count col not int64")?;
    Ok(col.value(0))
}

#[tokio::main]
async fn main() {
    let warehouse = format!(
        "{}/warehouse-rs",
        std::env::current_dir().unwrap().display()
    );
    std::fs::create_dir_all(&warehouse).ok();

    // 1. SQL catalog (SQLite file) on a local disk warehouse
    let base = std::env::current_dir().unwrap();
    // SqlCatalog connects without create_if_missing; the DB file must exist.
    let db_file = format!("{}/rs-catalog.db", base.display());
    if !std::path::Path::new(&db_file).exists() {
        std::fs::File::create(&db_file).ok();
    }
    let catalog_res: Result<Arc<dyn Catalog>, String> = async {
        let cat = SqlCatalogBuilder::default()
            .with_storage_factory(Arc::new(iceberg::io::LocalFsStorageFactory))
            .uri(format!("sqlite://{}/rs-catalog.db", base.display()))
            .warehouse_location(warehouse.clone())
            .load("spike", HashMap::new())
            .await
            .map_err(|e| e.to_string())?;
        Ok(Arc::new(cat) as Arc<dyn Catalog>)
    }
    .await;
    report("sql_catalog_sqlite", &catalog_res.as_ref().map(|_| "loaded".to_string()).map_err(|e| e.clone()));
    let catalog = match catalog_res {
        Ok(c) => c,
        Err(_) => return,
    };

    let ns = NamespaceIdent::new("db".to_string());
    let r = catalog
        .create_namespace(&ns, HashMap::new())
        .await
        .map(|_| "db".to_string())
        .map_err(|e| e.to_string());
    report("create_namespace", &r);

    // 2. Register with DataFusion
    let ctx = SessionContext::new();
    let reg = IcebergCatalogProvider::try_new(catalog.clone())
        .await
        .map_err(|e| e.to_string());
    report("df_catalog_provider", &reg.as_ref().map(|_| "built".to_string()).map_err(|e| e.clone()));
    let provider = match reg {
        Ok(p) => p,
        Err(_) => return,
    };
    ctx.register_catalog("ice", Arc::new(provider));

    // 3. CREATE TABLE through DataFusion SQL
    let r = ctx
        .sql("CREATE TABLE ice.db.events (id BIGINT NOT NULL, city VARCHAR, amount DOUBLE)")
        .await
        .map_err(|e| e.to_string());
    match r {
        Ok(df) => {
            let c = df.collect().await.map_err(|e| e.to_string());
            report("df_create_table", &c.map(|_| "created".to_string()));
        }
        Err(e) => report("df_create_table", &Err(e)),
    }
    let r = catalog
        .list_tables(&ns)
        .await
        .map(|t| format!("tables: {:?}", t))
        .map_err(|e| e.to_string());
    report("list_tables_after_ddl", &r);

    // 4. INSERT 100k via a registered in-memory source table
    ctx.register_batch("src1", batch(0, 100_000, "NYC", 1.5))
        .map_err(|e| e.to_string())
        .ok();
    let r = ctx
        .sql("INSERT INTO ice.db.events SELECT * FROM src1")
        .await
        .map_err(|e| e.to_string());
    match r {
        Ok(df) => {
            let c = df.collect().await.map_err(|e| e.to_string());
            report("df_insert_100k", &c.map(|_| "inserted".to_string()));
        }
        Err(e) => report("df_insert_100k", &Err(e)),
    }
    let r = sql_count(&ctx, "SELECT COUNT(*) FROM ice.db.events").await;
    report("df_count_after_write", &r.map(|c| c.to_string()));

    // snapshot id after first write (via iceberg API)
    let tid = iceberg::TableIdent::new(ns.clone(), "events".to_string());
    let snap1 = catalog
        .load_table(&tid)
        .await
        .map_err(|e| e.to_string())
        .and_then(|t| {
            t.metadata()
                .current_snapshot()
                .map(|s| s.snapshot_id())
                .ok_or("no snapshot".to_string())
        });
    report("snapshot1", &snap1.as_ref().map(|s| s.to_string()).map_err(|e| e.clone()));

    // 5. Append 50k
    ctx.register_batch("src2", batch(100_000, 50_000, "LA", 2.5))
        .map_err(|e| e.to_string())
        .ok();
    let r = ctx.sql("INSERT INTO ice.db.events SELECT * FROM src2").await;
    match r {
        Ok(df) => {
            let c = df.collect().await.map_err(|e| e.to_string());
            report("df_append_50k", &c.map(|_| "appended".to_string()));
        }
        Err(e) => report("df_append_50k", &Err(e.to_string())),
    }
    let r = sql_count(&ctx, "SELECT COUNT(*) FROM ice.db.events").await;
    report("df_count_after_append", &r.map(|c| c.to_string()));

    // 6. Time travel: static provider at snapshot 1 (iceberg API read as fallback proof)
    if let Ok(s1) = snap1 {
        let r = async {
            let t = catalog.load_table(&tid).await.map_err(|e| e.to_string())?;
            let scan = t
                .scan()
                .snapshot_id(s1)
                .build()
                .map_err(|e| e.to_string())?;
            let stream = scan.to_arrow().await.map_err(|e| e.to_string())?;
            use futures::StreamExt;
            let mut total = 0usize;
            let mut stream = Box::pin(stream);
            while let Some(b) = stream.next().await {
                total += b.map_err(|e| e.to_string())?.num_rows();
            }
            Ok::<String, String>(format!("rows at snapshot1 = {total}"))
        }
        .await;
        report("time_travel_iceberg_api", &r);
    }

    // 7. DELETE and UPDATE through DataFusion SQL
    let r = ctx.sql("DELETE FROM ice.db.events WHERE id = 5").await;
    match r {
        Ok(df) => {
            let c = df.collect().await.map_err(|e| e.to_string());
            report("df_delete", &c.map(|_| "delete executed".to_string()));
        }
        Err(e) => report("df_delete", &Err(e.to_string())),
    }
    let r = ctx
        .sql("UPDATE ice.db.events SET city = 'SF' WHERE id = 10")
        .await;
    match r {
        Ok(df) => {
            let c = df.collect().await.map_err(|e| e.to_string());
            report("df_update", &c.map(|_| "update executed".to_string()));
        }
        Err(e) => report("df_update", &Err(e.to_string())),
    }
    let r = sql_count(&ctx, "SELECT COUNT(*) FROM ice.db.events").await;
    report("df_count_final", &r.map(|c| c.to_string()));

    // 8. REST catalog client: build one (no server running; load should fail at network)
    let r = async {
        let rest = iceberg_catalog_rest::RestCatalogBuilder::default()
            .load(
                "rest-spike",
                HashMap::from([
                    ("uri".to_string(), "http://127.0.0.1:8181".to_string()),
                    ("warehouse".to_string(), warehouse.clone()),
                ]),
            )
            .await
            .map_err(|e| e.to_string())?;
        rest.list_namespaces(None)
            .await
            .map(|_| "listed".to_string())
            .map_err(|e| format!("client built; list failed as expected without server: {e}"))
    }
    .await;
    report("rest_catalog_client", &r);

    println!("DONE");
}
