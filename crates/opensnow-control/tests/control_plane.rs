use std::{collections::BTreeSet, sync::Arc, time::Instant};
use arrow::{array::Int64Array, record_batch::RecordBatch};
use opensnow_control::{ControlError, ControlPlane, ControlStore, CostRecordStatus, Role, TenantStatus, TokenScope};
use opensnow_storage::CatalogConfig;
use sqlx::ConnectOptions;

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

struct Fixture {
    plane: ControlPlane,
    storage: Arc<opensnow_storage::StorageCatalog>,
    db: String,
    _temp: tempfile::TempDir,
}

impl Fixture {
    async fn teardown(self) -> TestResult {
        let mut admin = sqlx::postgres::PgConnectOptions::new().host("localhost").port(5433).username("postgres").database("postgres").connect().await?;
        sqlx::query(&format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", self.db)).execute(&mut admin).await?;
        Ok(())
    }
}

fn ids(batches: &[RecordBatch]) -> BTreeSet<i64> {
    batches.iter().flat_map(|b| b.column(0).as_any().downcast_ref::<Int64Array>().unwrap().values().to_vec()).collect()
}

async fn setup(label: &str) -> Result<Option<Fixture>, Box<dyn std::error::Error + Send + Sync>> {
    let mut admin = match sqlx::postgres::PgConnectOptions::new().host("localhost").port(5433).username("postgres").database("postgres").connect().await {
        Ok(c) => c,
        Err(error) => {
            let unreachable = matches!(&error, sqlx::Error::Io(io) if matches!(io.kind(),
                std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::TimedOut |
                std::io::ErrorKind::NetworkUnreachable | std::io::ErrorKind::HostUnreachable));
            if unreachable {
                println!("SKIP control plane {label}: Postgres unreachable: {error}");
                return Ok(None);
            }
            return Err(error.into());
        }
    };
    let suffix = uuid::Uuid::now_v7().simple().to_string();
    let db = format!("opensnow_ctrl_{label}_{}", &suffix[..8]);
    sqlx::query(&format!("DROP DATABASE IF EXISTS {db} WITH (FORCE)")).execute(&mut admin).await?;
    sqlx::query(&format!("CREATE DATABASE {db}")).execute(&mut admin).await?;
    let uri = format!("postgres://postgres@localhost:5433/{db}");
    let pool = sqlx::PgPool::connect(&uri).await?;
    let store = ControlStore::new(pool);
    store.ensure_schema().await?;
    store.ensure_schema().await?;
    let temp = tempfile::tempdir()?;
    let storage = CatalogConfig::postgres("control-test", uri.clone(), temp.path().join("warehouse")).connect().await
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e })?;
    let storage = Arc::new(storage);
    let plane = ControlPlane::new(store, storage.clone());
    Ok(Some(Fixture { plane, storage, db, _temp: temp }))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provisioning_creates_tenant_admin_and_token_under_five_seconds() -> TestResult {
    let Some(fx) = setup("prov").await? else { return Ok(()); };
    let start = Instant::now();
    let p = fx.plane.provision_tenant("acme", "admin@acme.test", "s3cret-admin-pass", Some(1_000_000), None).await?;
    let elapsed = start.elapsed();
    println!("provisioning elapsed: {elapsed:?}");
    assert!(elapsed.as_secs() < 5, "provisioning took {elapsed:?}");
    assert_eq!(p.tenant.name, "acme");
    assert_eq!(p.tenant.status, TenantStatus::Active);
    assert_eq!(p.admin_user.login, "admin@acme.test");
    assert_eq!(p.admin_user.role, Role::Admin);
    let stored = fx.plane.store().stored_token_hash(&p.tenant.id, &p.token_record.id).await?.expect("token row");
    assert_ne!(stored, p.token);
    assert!(!stored.contains(&p.token));
    assert!(fx.plane.authenticate_token(&p.token).await.is_ok());
    assert!(fx.plane.is_registered(&p.tenant.id).await);
    fx.teardown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auth_token_and_password_outcomes() -> TestResult {
    let Some(fx) = setup("auth").await? else { return Ok(()); };
    let p = fx.plane.provision_tenant("acme", "admin@acme.test", "s3cret-admin-pass", None, None).await?;
    assert!(fx.plane.authenticate_token(&p.token).await.is_ok());
    assert!(matches!(fx.plane.authenticate_token("osn_wrong").await, Err(ControlError::AuthFailed)));
    assert!(matches!(fx.plane.authenticate_token("").await, Err(ControlError::AuthFailed)));
    assert!(fx.plane.authenticate_password("acme", "admin@acme.test", "s3cret-admin-pass").await.is_ok());
    assert!(matches!(fx.plane.authenticate_password("acme", "admin@acme.test", "wrong-pass").await, Err(ControlError::AuthFailed)));
    assert!(matches!(fx.plane.authenticate_password("acme", "ghost@acme.test", "s3cret-admin-pass").await, Err(ControlError::AuthFailed)));
    let short = fx.plane.issue_token(&p.tenant.id, &p.admin_user.id, vec![TokenScope::Read], Some(3600)).await?;
    assert!(fx.plane.authenticate_token(&short.token).await.is_ok());
    fx.plane.revoke_token(&p.tenant.id, &short.record.id).await?;
    assert!(matches!(fx.plane.authenticate_token(&short.token).await, Err(ControlError::AuthFailed)));
    let expired = fx.plane.issue_token(&p.tenant.id, &p.admin_user.id, vec![TokenScope::Read], Some(-3600)).await?;
    assert!(matches!(fx.plane.authenticate_token(&expired.token).await, Err(ControlError::AuthFailed)));
    let read_only = fx.plane.issue_token(&p.tenant.id, &p.admin_user.id, vec![], None).await?;
    assert_eq!(read_only.record.scopes, vec![TokenScope::Read]);
    assert!(matches!(
        fx.plane.execute_authenticated(&read_only.token, "CREATE TABLE events (id BIGINT NOT NULL)").await,
        Err(ControlError::AccessDenied(_))
    ));
    fx.teardown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn isolation_between_tenants() -> TestResult {
    let Some(fx) = setup("iso").await? else { return Ok(()); };
    let a = fx.plane.provision_tenant("tenant-a", "admin@a.test", "pass-a-admin", None, None).await?;
    let b = fx.plane.provision_tenant("tenant-b", "admin@b.test", "pass-b-admin", None, None).await?;
    fx.plane.execute_authenticated(&a.token, "CREATE TABLE events (id BIGINT NOT NULL)").await?;
    fx.plane.execute_authenticated(&a.token, "INSERT INTO events VALUES (1), (2), (3)").await?;
    fx.plane.execute_authenticated(&b.token, "CREATE TABLE events (id BIGINT NOT NULL)").await?;
    fx.plane.execute_authenticated(&b.token, "INSERT INTO events VALUES (7), (8)").await?;
    let a_rows = fx.plane.execute_authenticated(&a.token, "SELECT id FROM events").await?;
    assert_eq!(ids(&a_rows.batches), BTreeSet::from([1, 2, 3]));
    let b_rows = fx.plane.execute_authenticated(&b.token, "SELECT id FROM events").await?;
    assert_eq!(ids(&b_rows.batches), BTreeSet::from([7, 8]));
    assert_eq!(fx.plane.list_tables(&a.token).await?, vec!["events".to_string()]);
    assert_eq!(fx.plane.list_tables(&b.token).await?, vec!["events".to_string()]);
    let cross = format!("SELECT id FROM ice.{}.events", a.tenant.namespace);
    assert!(fx.plane.execute_authenticated(&b.token, &cross).await.is_err());
    let bare_cross = fx.plane.execute_authenticated(&b.token, &format!("SELECT id FROM {}.events", a.tenant.namespace)).await;
    assert!(bare_cross.is_err());
    assert!(matches!(fx.plane.execute_authenticated(&a.token, &format!("SELECT id FROM ice.{}.events", b.tenant.namespace)).await, Err(ControlError::AccessDenied(_))));
    fx.teardown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn quoted_identifiers_do_not_bypass_tenant_isolation() -> TestResult {
    // Issue #23 audit reproduction, end to end: before the fix, the
    // quoted forms below passed the guard unchanged and executed
    // against the shared catalog.
    let Some(fx) = setup("quote").await? else { return Ok(()); };
    let a = fx.plane.provision_tenant("tenant-a", "admin@a.test", "pass-a-admin", None, None).await?;
    let b = fx.plane.provision_tenant("tenant-b", "admin@b.test", "pass-b-admin", None, None).await?;
    fx.plane.execute_authenticated(&a.token, "CREATE TABLE events (id BIGINT NOT NULL)").await?;
    fx.plane.execute_authenticated(&a.token, "INSERT INTO events VALUES (1), (2), (3)").await?;
    fx.plane.execute_authenticated(&b.token, "CREATE TABLE events (id BIGINT NOT NULL)").await?;
    fx.plane.execute_authenticated(&b.token, "INSERT INTO events VALUES (7), (8)").await?;
    // Quoted cross-tenant read, the audit's exact shape.
    let quoted_cross = format!(r#"SELECT id FROM "ice"."{}"."events""#, a.tenant.namespace);
    assert!(matches!(fx.plane.execute_authenticated(&b.token, &quoted_cross).await, Err(ControlError::AccessDenied(_))));
    // Quoted read of the caller's own namespace is also rejected:
    // the guard never emits quoted names.
    let quoted_own = format!(r#"SELECT id FROM "ice"."{}"."events""#, b.tenant.namespace);
    assert!(matches!(fx.plane.execute_authenticated(&b.token, &quoted_own).await, Err(ControlError::AccessDenied(_))));
    // A quoted branch inside a UNION ALL does not slip through.
    let quoted_union = format!(r#"SELECT id FROM events UNION ALL SELECT id FROM "ice"."{}"."events""#, a.tenant.namespace);
    assert!(matches!(fx.plane.execute_authenticated(&b.token, &quoted_union).await, Err(ControlError::AccessDenied(_))));
    // Both tenants' plain reads still work and stay separate.
    let a_rows = fx.plane.execute_authenticated(&a.token, "SELECT id FROM events").await?;
    assert_eq!(ids(&a_rows.batches), BTreeSet::from([1, 2, 3]));
    let b_rows = fx.plane.execute_authenticated(&b.token, "SELECT id FROM events").await?;
    assert_eq!(ids(&b_rows.batches), BTreeSet::from([7, 8]));
    fx.teardown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn comma_joins_do_not_bypass_tenant_isolation() -> TestResult {
    // Issue #22 audit reproduction, end to end: before the fix, a
    // comma separated FROM list let a second table reference pass the
    // guard unchecked, so a cross-tenant read and an INSERT INTO ...
    // SELECT copy both succeeded against the shared catalog.
    let Some(fx) = setup("comma").await? else { return Ok(()); };
    let a = fx.plane.provision_tenant("tenant-a", "admin@a.test", "pass-a-admin", None, None).await?;
    let b = fx.plane.provision_tenant("tenant-b", "admin@b.test", "pass-b-admin", None, None).await?;
    fx.plane.execute_authenticated(&a.token, "CREATE TABLE events (id BIGINT NOT NULL)").await?;
    fx.plane.execute_authenticated(&a.token, "INSERT INTO events VALUES (1), (2), (3)").await?;
    fx.plane.execute_authenticated(&b.token, "CREATE TABLE events (id BIGINT NOT NULL)").await?;
    fx.plane.execute_authenticated(&b.token, "INSERT INTO events VALUES (7), (8)").await?;
    fx.plane.execute_authenticated(&b.token, "CREATE TABLE notes (id BIGINT NOT NULL)").await?;
    fx.plane.execute_authenticated(&b.token, "INSERT INTO notes VALUES (7)").await?;
    // Cross-tenant comma reads, three part and two part forms.
    let cross = format!("SELECT id FROM notes, ice.{}.events", a.tenant.namespace);
    assert!(matches!(fx.plane.execute_authenticated(&b.token, &cross).await, Err(ControlError::AccessDenied(_))));
    let cross_bare = format!("SELECT id FROM notes, {}.events", a.tenant.namespace);
    assert!(matches!(fx.plane.execute_authenticated(&b.token, &cross_bare).await, Err(ControlError::AccessDenied(_))));
    // The exfiltration copy is denied and no rows move.
    let exfil = format!("INSERT INTO events SELECT id FROM notes, ice.{}.events", a.tenant.namespace);
    assert!(matches!(fx.plane.execute_authenticated(&b.token, &exfil).await, Err(ControlError::AccessDenied(_))));
    let b_rows = fx.plane.execute_authenticated(&b.token, "SELECT id FROM events").await?;
    assert_eq!(ids(&b_rows.batches), BTreeSet::from([7, 8]));
    // The victim still reads its own rows, untouched.
    let a_rows = fx.plane.execute_authenticated(&a.token, "SELECT id FROM events").await?;
    assert_eq!(ids(&a_rows.batches), BTreeSet::from([1, 2, 3]));
    // An own tables comma join returns the joined rows.
    let joined = fx.plane.execute_authenticated(&b.token, "SELECT events.id FROM events, notes WHERE events.id = notes.id").await?;
    assert_eq!(ids(&joined.batches), BTreeSet::from([7]));
    fx.teardown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn suspend_blocks_and_unsuspend_restores() -> TestResult {
    let Some(fx) = setup("susp").await? else { return Ok(()); };
    let a = fx.plane.provision_tenant("tenant-a", "admin@a.test", "pass-a-admin", None, None).await?;
    let b = fx.plane.provision_tenant("tenant-b", "admin@b.test", "pass-b-admin", None, None).await?;
    fx.plane.execute_authenticated(&a.token, "CREATE TABLE events (id BIGINT NOT NULL)").await?;
    fx.plane.execute_authenticated(&a.token, "INSERT INTO events VALUES (42)").await?;
    fx.plane.suspend_tenant(&a.tenant.id).await?;
    assert!(matches!(fx.plane.authenticate_token(&a.token).await, Err(ControlError::AuthFailed)));
    assert!(fx.plane.execute_authenticated(&a.token, "SELECT id FROM events").await.is_err());
    assert!(fx.plane.authenticate_token(&b.token).await.is_ok());
    fx.plane.unsuspend_tenant(&a.tenant.id).await?;
    let rows = fx.plane.execute_authenticated(&a.token, "SELECT id FROM events").await?;
    assert_eq!(ids(&rows.batches), BTreeSet::from([42]));
    fx.teardown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_removes_access_and_engine_registration() -> TestResult {
    let Some(fx) = setup("del").await? else { return Ok(()); };
    let a = fx.plane.provision_tenant("tenant-a", "admin@a.test", "pass-a-admin", None, None).await?;
    let b = fx.plane.provision_tenant("tenant-b", "admin@b.test", "pass-b-admin", None, None).await?;
    fx.plane.execute_authenticated(&b.token, "CREATE TABLE events (id BIGINT NOT NULL)").await?;
    fx.plane.delete_tenant(&a.tenant.id).await?;
    assert!(matches!(fx.plane.authenticate_token(&a.token).await, Err(ControlError::AuthFailed)));
    assert!(!fx.plane.is_registered(&a.tenant.id).await);
    let gone = fx.plane.store().get_tenant_by_id(&a.tenant.id).await?.expect("tombstone");
    assert_eq!(gone.status, TenantStatus::Deleted);
    assert!(fx.plane.authenticate_token(&b.token).await.is_ok());
    fx.plane.execute_authenticated(&b.token, "INSERT INTO events VALUES (9)").await?;
    let rows = fx.plane.execute_authenticated(&b.token, "SELECT id FROM events").await?;
    assert_eq!(ids(&rows.batches), BTreeSet::from([9]));
    fx.teardown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cost_cap_enforced_through_authenticated_path() -> TestResult {
    let Some(fx) = setup("caps").await? else { return Ok(()); };
    let p = fx.plane.provision_tenant("capped", "admin@capped.test", "pass-capped", Some(2), None).await?;
    fx.plane.execute_authenticated(&p.token, "CREATE TABLE events (id BIGINT NOT NULL)").await?;
    fx.plane.execute_authenticated(&p.token, "INSERT INTO events VALUES (1), (2), (3)").await?;
    let err = fx.plane.execute_authenticated(&p.token, "SELECT id FROM events").await.unwrap_err();
    assert!(matches!(err, ControlError::CostCapExceeded), "{err}");
    fx.teardown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn duplicate_tenant_name_rejected_and_original_unaffected() -> TestResult {
    let Some(fx) = setup("dup").await? else { return Ok(()); };
    let p = fx.plane.provision_tenant("acme", "admin@acme.test", "s3cret-admin-pass", None, None).await?;
    let dup = fx.plane.provision_tenant("acme", "other@acme.test", "other-pass-123", None, None).await;
    assert!(matches!(dup, Err(ControlError::DuplicateTenant)));
    assert!(fx.plane.authenticate_token(&p.token).await.is_ok());
    let member = fx.plane.create_user(&p.tenant.id, "dev@acme.test", "dev-pass-123", Role::Member).await?;
    assert_eq!(member.role, Role::Member);
    assert!(matches!(
        fx.plane.create_user(&p.tenant.id, "dev@acme.test", "dev-pass-456", Role::Member).await,
        Err(ControlError::DuplicateUser)
    ));
    fx.teardown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn metering_records_read_back_in_dollars_per_tenant() -> TestResult {
    let Some(fx) = setup("meter").await? else { return Ok(()); };
    let a = fx.plane.provision_tenant("acme", "admin@acme.test", "s3cret-admin-pass", None, None).await?;
    let b = fx.plane.provision_tenant("other", "admin@other.test", "s3cret-other-pass", None, None).await?;
    fx.plane.execute_authenticated(&a.token, "CREATE TABLE events (id BIGINT NOT NULL)").await?;
    fx.plane.execute_authenticated(&a.token, "INSERT INTO events VALUES (1), (2), (3)").await?;
    let selected = fx.plane.execute_authenticated(&a.token, "SELECT id FROM events").await?;
    assert_eq!(selected.outcome.rows_returned, 3);
    assert!(selected.outcome.cost_dollars >= 0.0);
    // A query that fails inside the engine still executed and is metered.
    assert!(fx.plane.execute_authenticated(&a.token, "SELECT id FROM missing_table").await.is_err());

    let records = fx.plane.cost_records(&a.token).await?;
    assert_eq!(records.len(), 4);
    assert_eq!(records.iter().map(|r| r.seq).collect::<Vec<_>>(), vec![1, 2, 3, 4]);
    assert_eq!(records[2].status, CostRecordStatus::Completed);
    assert_eq!(records[2].rows_returned, 3);
    assert_eq!(records[3].status, CostRecordStatus::Failed);
    for record in &records {
        // Dollar math from docs/METERING.md: vCPU-seconds at the default
        // internal cost rate of $0.0504 per vCPU-hour (COST-MODEL.md).
        let expected = record.vcpu_seconds / 3600.0 * 0.0504;
        assert!((record.cost_dollars - expected).abs() < 1e-12, "{record:?}");
        assert_eq!(record.tenant, a.tenant.id);
    }
    // Read-back is idempotent: a second read syncs nothing twice.
    assert_eq!(fx.plane.cost_records(&a.token).await?.len(), 4);
    // Durability: the rows are in Postgres, not only in the engine.
    let stored = fx.plane.store().list_cost_records(&a.tenant.id).await?;
    assert_eq!(stored.len(), 4);
    assert_eq!(stored[2].rows_returned, 3);

    let summary = fx.plane.cost_summary(&a.token).await?;
    assert_eq!(summary.record_count, 4);
    assert_eq!(summary.total_rows_returned, records.iter().map(|r| r.rows_returned).sum::<u64>());
    let expected_dollars: f64 = records.iter().map(|r| r.cost_dollars).sum();
    assert!((summary.total_cost_dollars - expected_dollars).abs() < 1e-9);

    // The other tenant sees none of this, and a bad token fails opaque.
    assert!(fx.plane.cost_records(&b.token).await?.is_empty());
    assert_eq!(fx.plane.cost_summary(&b.token).await?.record_count, 0);
    assert!(matches!(fx.plane.cost_records("osn_not_a_real_token").await, Err(ControlError::AuthFailed)));
    assert!(matches!(fx.plane.cost_summary("osn_not_a_real_token").await, Err(ControlError::AuthFailed)));
    fx.teardown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mutations_use_writer_snapshots_and_metering() -> TestResult {
    let Some(fx) = setup("mutations").await? else { return Ok(()); };
    let p = fx.plane.provision_tenant("mutations", "admin@mutations.test", "s3cret-admin-pass", None, None).await?;
    let storage = &fx.storage;
    let namespace = p.tenant.namespace.as_str();
    for sql in [
        "CREATE TABLE events (id BIGINT NOT NULL, value BIGINT)",
        "INSERT INTO events VALUES (1, 10), (2, 20), (3, NULL)",
        "INSERT INTO events VALUES (4, 40), (5, 50)",
        "CREATE TABLE incoming (id BIGINT NOT NULL, value BIGINT)",
        "INSERT INTO incoming VALUES (2, 200), (3, 300), (6, 60)",
    ] { fx.plane.execute_authenticated(&p.token, sql).await?; }
    let ident = iceberg::TableIdent::from_strs([namespace, "events"])?;
    let cases: &[(&str, &[(i64, i64)])] = &[
        ("MERGE INTO events AS e USING incoming AS i ON e.id = i.id WHEN MATCHED THEN UPDATE SET e.value = i.value WHEN NOT MATCHED THEN INSERT (id, value) VALUES (i.id, i.value)", &[(1, 10), (2, 200), (3, 300), (4, 40), (5, 50), (6, 60)]),
        ("MERGE INTO events AS e USING incoming AS i ON e.id = i.id WHEN MATCHED AND e.value >= 300 THEN DELETE WHEN MATCHED THEN UPDATE SET e.value = i.value", &[(1, 10), (2, 200), (4, 40), (5, 50), (6, 60)]),
        ("UPDATE events AS e SET value = i.value + 1 FROM incoming AS i WHERE e.id = i.id", &[(1, 10), (2, 201), (4, 40), (5, 50), (6, 61)]),
        ("UPDATE events SET value = value + 1 WHERE id = 1", &[(1, 11), (2, 201), (4, 40), (5, 50), (6, 61)]),
        ("DELETE FROM events WHERE id = 5", &[(1, 11), (2, 201), (4, 40), (6, 61)]),
        ("MERGE INTO events AS e USING incoming AS i ON e.id = i.id WHEN MATCHED AND e.id = 2 THEN UPDATE SET e.value = i.value WHEN MATCHED AND e.id = 6 THEN DELETE WHEN NOT MATCHED THEN INSERT (id, value) VALUES (i.id, i.value)", &[(1, 11), (2, 200), (3, 300), (4, 40)]),
    ];
    for (sql, expected) in cases {
        let before = storage.snapshots(&ident).await?;
        let result = fx.plane.execute_authenticated(&p.token, sql).await?;
        assert!(result.batches.is_empty());
        assert_eq!(result.outcome.cost_units_charged, 0);
        let records = fx.plane.cost_records(&p.token).await?;
        let record = records.last().unwrap();
        assert_eq!(record.seq, result.outcome.cost_record_seq);
        assert_eq!(record.status, opensnow_engine::CostRecordStatus::Completed);
        assert_eq!(record.rows_returned, 0);
        assert_eq!(record.bytes_returned, 0);
        assert_eq!(record.elapsed, result.outcome.elapsed);
        assert_eq!(storage.snapshots(&ident).await?.len(), before.len() + 1, "{sql}");
        let rows = fx.plane.execute_authenticated(&p.token, "SELECT id, value FROM events ORDER BY id").await?;
        let actual: Vec<(i64, i64)> = rows.batches.iter().flat_map(|batch| {
            let ids = batch.column(0).as_any().downcast_ref::<arrow::array::Int64Array>().unwrap();
            let values = batch.column(1).as_any().downcast_ref::<arrow::array::Int64Array>().unwrap();
            (0..batch.num_rows()).map(|i| (ids.value(i), values.value(i))).collect::<Vec<_>>()
        }).collect();
        assert_eq!(&actual, expected, "{sql}");
    }
    let before = storage.snapshots(&ident).await?;
    for sql in ["UPDATE events SET value = 0 WHERE id > 100", "DELETE FROM events WHERE id > 100",
        "MERGE INTO events AS e USING incoming AS i ON false WHEN MATCHED THEN DELETE"] {
        fx.plane.execute_authenticated(&p.token, sql).await?;
        assert_eq!(storage.snapshots(&ident).await?, before);
    }
    let original = fx.plane.execute_authenticated(&p.token, "SELECT id, value FROM events ORDER BY id").await?.batches;
    fx.plane.execute_authenticated(&p.token, "INSERT INTO incoming VALUES (2, 201)").await?;
    let error = fx.plane.execute_authenticated(&p.token, "MERGE INTO events AS e USING incoming AS i ON e.id = i.id WHEN MATCHED THEN UPDATE SET e.value = i.value").await.unwrap_err();
    assert!(error.to_string().contains("more than once"), "{error}");
    assert_eq!(storage.snapshots(&ident).await?, before);
    let records = fx.plane.cost_records(&p.token).await?;
    assert_eq!(records.last().unwrap().status, opensnow_engine::CostRecordStatus::Failed);
    assert_eq!(fx.plane.execute_authenticated(&p.token, "SELECT id, value FROM events ORDER BY id").await?.batches, original);
    let other = fx.plane.provision_tenant("other", "admin@other.test", "s3cret-admin-pass", None, None).await?;
    for sql in [
        format!("MERGE INTO {}.events AS e USING incoming AS i ON e.id = i.id WHEN MATCHED THEN DELETE", other.tenant.namespace),
        format!("MERGE INTO events AS e USING {}.incoming AS i ON e.id = i.id WHEN MATCHED THEN DELETE", other.tenant.namespace),
        format!("UPDATE {}.events SET value = 0", other.tenant.namespace),
        format!("DELETE FROM {}.events", other.tenant.namespace),
    ] {
        assert!(matches!(fx.plane.execute_authenticated(&p.token, &sql).await, Err(ControlError::AccessDenied(_))));
    }
    fx.teardown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mutation_checks_cap_before_storage_runs() -> TestResult {
    let Some(fx) = setup("mutationcap").await? else { return Ok(()); };
    let p = fx.plane.provision_tenant("mutationcap", "admin@cap.test", "s3cret-admin-pass", Some(0), None).await?;
    for sql in [
        "DELETE FROM missing WHERE id = 1",
        "UPDATE missing SET id = 1",
        "MERGE INTO missing AS e USING incoming AS i ON e.id = i.id WHEN MATCHED THEN DELETE",
    ] {
        assert!(matches!(fx.plane.execute_authenticated(&p.token, sql).await,
            Err(ControlError::CostCapExceeded)));
    }
    assert!(fx.plane.cost_records(&p.token).await?.is_empty());
    fx.teardown().await
}

/// A fresh ControlPlane over the same Postgres database and the same
/// warehouse directory. This is a hosted restart: its engine starts
/// empty, so no tenant is registered in it.
async fn fresh_plane(fx: &Fixture) -> Result<ControlPlane, Box<dyn std::error::Error + Send + Sync>> {
    let uri = format!("postgres://postgres@localhost:5433/{}", fx.db);
    let pool = sqlx::PgPool::connect(&uri).await?;
    let store = ControlStore::new(pool);
    store.ensure_schema().await?;
    let storage = CatalogConfig::postgres("control-test", uri.clone(), fx._temp.path().join("warehouse")).connect().await
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e })?;
    Ok(ControlPlane::new(store, Arc::new(storage)))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tenants_survive_a_hosted_restart() -> TestResult {
    let Some(fx) = setup("restart").await? else { return Ok(()); };
    let a = fx.plane.provision_tenant("tenant-a", "admin@a.test", "pass-a-admin", None, None).await?;
    let b = fx.plane.provision_tenant("tenant-b", "admin@b.test", "pass-b-admin", None, None).await?;
    fx.plane.execute_authenticated(&a.token, "CREATE TABLE events (id BIGINT NOT NULL, value BIGINT)").await?;
    fx.plane.execute_authenticated(&a.token, "INSERT INTO events VALUES (1, 10), (2, 20)").await?;
    fx.plane.execute_authenticated(&b.token, "CREATE TABLE events (id BIGINT NOT NULL, value BIGINT)").await?;
    fx.plane.execute_authenticated(&b.token, "INSERT INTO events VALUES (7, 70)").await?;
    let c = fx.plane.provision_tenant("tenant-c", "admin@c.test", "pass-c-admin", None, None).await?;
    fx.plane.suspend_tenant(&c.tenant.id).await?;
    // The pre-restart cost records are durable in Postgres.
    assert_eq!(fx.plane.store().list_cost_records(&a.tenant.id).await?.len(), 2);

    // Restart: a fresh plane over the same store and warehouse.
    let plane2 = fresh_plane(&fx).await?;
    assert!(!plane2.is_registered(&a.tenant.id).await);
    assert!(!plane2.is_registered(&b.tenant.id).await);

    // The original token runs SQL with no re-provisioning, and the
    // row written before the restart reads back.
    let selected = plane2.execute_authenticated(&a.token, "SELECT id FROM events").await?;
    assert_eq!(ids(&selected.batches), BTreeSet::from([1, 2]));
    assert_eq!(selected.outcome.cost_record_seq, 3);
    assert!(plane2.is_registered(&a.tenant.id).await);

    // Writes work after the restart, including a storage-writer
    // mutation, which produces exactly one cost record: its sequence
    // advances by one from the insert before it.
    let inserted = plane2.execute_authenticated(&a.token, "INSERT INTO events VALUES (3, 30)").await?;
    let updated = plane2.execute_authenticated(&a.token, "UPDATE events SET value = value + 1 WHERE id = 1").await?;
    assert_eq!(updated.outcome.cost_record_seq, inserted.outcome.cost_record_seq + 1);
    let rows = plane2.execute_authenticated(&a.token, "SELECT id, value FROM events ORDER BY id").await?;
    let actual: Vec<(i64, i64)> = rows.batches.iter().flat_map(|batch| {
        let ids_col = batch.column(0).as_any().downcast_ref::<Int64Array>().unwrap();
        let values = batch.column(1).as_any().downcast_ref::<Int64Array>().unwrap();
        (0..batch.num_rows()).map(|i| (ids_col.value(i), values.value(i))).collect::<Vec<_>>()
    }).collect();
    assert_eq!(actual, vec![(1, 11), (2, 20), (3, 30)]);

    // The second pre-restart tenant also self-heals, and isolation
    // holds: it sees only its own rows.
    let b_rows = plane2.execute_authenticated(&b.token, "SELECT id FROM events").await?;
    assert_eq!(ids(&b_rows.batches), BTreeSet::from([7]));
    assert!(plane2.is_registered(&b.tenant.id).await);

    // Cost records written before the restart still read back.
    assert_eq!(plane2.cost_records(&a.token).await?.len(), 6);

    // A tenant suspended before the restart stays dead after it and
    // is never lazily registered.
    assert!(matches!(plane2.execute_authenticated(&c.token, "SELECT id FROM events").await, Err(ControlError::AuthFailed)));
    assert!(!plane2.is_registered(&c.tenant.id).await);
    fx.teardown().await
}
