//! Local mode: the same engine, catalog, and SQL guard as hosted,
//! embedded in one process with no account, no credentials, and no
//! Postgres. State lives under one directory: a SQLite catalog file
//! and a local warehouse. Reopening the directory reopens the database.

use std::sync::Arc;

use iceberg::NamespaceIdent;
use iceberg_datafusion::IcebergCatalogProvider;
use opensnow_control::{guard_sql, Access};
use opensnow_engine::{CostRecord, EngineService, QueryResult, ResourceGroup, TenantCostSummary};
use opensnow_storage::{CatalogConfig, StorageCatalog};
use tokio::sync::Mutex;

pub const LOCAL_TENANT: &str = "local";
pub const LOCAL_NAMESPACE: &str = "local";

type BoxError = Box<dyn std::error::Error + Send + Sync>;

pub struct LocalDb {
    engine: Arc<Mutex<EngineService>>,
    storage: Arc<StorageCatalog>,
    pub root: std::path::PathBuf,
}

impl LocalDb {
    pub async fn open(root: impl Into<std::path::PathBuf>, cost_cap_units: Option<u64>) -> Result<Self, BoxError> {
        let root = root.into();
        std::fs::create_dir_all(&root)?;
        let storage = Arc::new(
            CatalogConfig::sqlite("opensnow-local", root.join("catalog.db"), root.join("warehouse"))?
                .connect().await?,
        );
        let namespace = NamespaceIdent::new(LOCAL_NAMESPACE.into());
        if storage.create_namespace(&namespace).await.is_err() {
            // Reopening an existing database lands here. Confirm the
            // namespace is really there; any other failure propagates
            // as the list error instead of being hidden.
            let namespaces = storage.catalog.list_namespaces(None).await.map_err(|e| -> BoxError { Box::new(e) })?;
            if !namespaces.iter().any(|n| n.to_string() == LOCAL_NAMESPACE) {
                return Err("local namespace missing after create failure".into());
            }
        }
        let mut engine = EngineService::new();
        engine.register_tenant(LOCAL_TENANT, ResourceGroup { cost_cap_units, ..ResourceGroup::default() })?;
        let provider = IcebergCatalogProvider::try_new(storage.catalog.clone()).await.map_err(|e| -> BoxError { Box::new(e) })?;
        engine.context(LOCAL_TENANT)?.register_catalog("ice", Arc::new(provider));
        Ok(Self { engine: Arc::new(Mutex::new(engine)), storage, root })
    }

    /// Runs one statement through the hosted guard, so table names are
    /// qualified and statement rules match the authenticated path.
    pub async fn execute(&self, sql: &str) -> Result<QueryResult, BoxError> {
        let (rewritten, _access) = guard_sql(LOCAL_NAMESPACE, sql).map_err(|e| -> BoxError { e.to_string().into() })?;
        let engine = self.engine.lock().await;
        Ok(engine.execute(LOCAL_TENANT, &rewritten).await?)
    }

    pub fn classify(&self, sql: &str) -> Result<Access, BoxError> {
        let (_rewritten, access) = guard_sql(LOCAL_NAMESPACE, sql).map_err(|e| -> BoxError { e.to_string().into() })?;
        Ok(access)
    }

    pub async fn list_tables(&self) -> Result<Vec<String>, BoxError> {
        let namespace = NamespaceIdent::new(LOCAL_NAMESPACE.into());
        let idents = self.storage.catalog.list_tables(&namespace).await.map_err(|e| -> BoxError { Box::new(e) })?;
        let mut names: Vec<String> = idents.iter().map(|t| t.name().to_string()).collect();
        names.sort();
        Ok(names)
    }

    pub async fn describe_table(&self, table: &str) -> Result<Vec<(String, String)>, BoxError> {
        if table.is_empty() || !table.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err("bad table name".into());
        }
        let ident = iceberg::TableIdent::new(NamespaceIdent::new(LOCAL_NAMESPACE.into()), table.into());
        let loaded = self.storage.catalog.load_table(&ident).await.map_err(|e| -> BoxError { Box::new(e) })?;
        let schema = loaded.metadata().current_schema();
        Ok(schema.as_struct().fields().iter().map(|f| (f.name.clone(), type_name(&f.field_type))).collect())
    }

    pub async fn cost_records(&self) -> Result<Vec<CostRecord>, BoxError> {
        let engine = self.engine.lock().await;
        Ok(engine.cost_records(LOCAL_TENANT)?)
    }

    pub async fn cost_summary(&self) -> Result<TenantCostSummary, BoxError> {
        let engine = self.engine.lock().await;
        Ok(engine.cost_summary(LOCAL_TENANT)?)
    }
}

fn type_name(t: &iceberg::spec::Type) -> String {
    use iceberg::spec::{PrimitiveType, Type};
    match t {
        Type::Primitive(p) => match p {
            PrimitiveType::Boolean => "BOOLEAN".into(),
            PrimitiveType::Int => "INT".into(),
            PrimitiveType::Long => "BIGINT".into(),
            PrimitiveType::Float => "FLOAT".into(),
            PrimitiveType::Double => "DOUBLE".into(),
            PrimitiveType::String => "VARCHAR".into(),
            PrimitiveType::Date => "DATE".into(),
            PrimitiveType::Timestamp => "TIMESTAMP".into(),
            PrimitiveType::Timestamptz => "TIMESTAMP WITH TIME ZONE".into(),
            PrimitiveType::Binary => "BINARY".into(),
            PrimitiveType::Uuid => "UUID".into(),
            other => format!("{other}"),
        },
        other => format!("{other}"),
    }
}
