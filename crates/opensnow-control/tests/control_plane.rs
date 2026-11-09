use std::{collections::BTreeSet, sync::Arc, time::Instant};
use arrow::{array::Int64Array, record_batch::RecordBatch};
use opensnow_control::{ControlError, ControlPlane, ControlStore, Role, TenantStatus, TokenScope};
use opensnow_storage::CatalogConfig;
use sqlx::ConnectOptions;

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

struct Fixture {
    plane: ControlPlane,
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
    let plane = ControlPlane::new(store, Arc::new(storage));
    Ok(Some(Fixture { plane, db, _temp: temp }))
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
