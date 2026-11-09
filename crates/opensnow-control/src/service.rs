//! Tenant lifecycle and the authenticated execute path. One auth check is
//! shared by the in-process SQL endpoint and any future UI caller. There is
//! no HTTP transport in this slice.
use std::{sync::Arc, time::Duration};
use iceberg::{Catalog, NamespaceIdent};
use iceberg_datafusion::IcebergCatalogProvider;
use opensnow_engine::{EngineService, QueryResult, ResourceGroup};
use opensnow_storage::StorageCatalog;
use sqlparser::{dialect::GenericDialect, parser::Parser};
use tokio::sync::Mutex;
use crate::credentials;
use crate::store::{self, ControlStore, NewTenant, NewToken, NewUser};
use crate::types::{ControlError, IssuedToken, Principal, ProvisionedTenant, Result, Role, TenantRecord, TenantStatus, TokenScope};

const DEFAULT_TOKEN_TTL_SECONDS: i64 = 24 * 60 * 60;

pub fn namespace_for_tenant(tenant_id: &str) -> String {
    let compact: String = tenant_id.chars().filter(|c| *c != '-').collect();
    format!("t{}", &compact[..compact.len().min(16)])
}

fn valid_ident(s: &str) -> bool {
    !s.is_empty() && s.len() <= 63
        && s.chars().next().map(|c| c.is_ascii_alphabetic() || c == '_').unwrap_or(false)
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Access { Read, Write }

/// Qualifies every table reference to the tenant namespace and rejects any
/// reference outside it. Fail closed: quoted identifiers, comments, CTE
/// names, and multi-statement input are rejected or fail at execution.
fn guard_sql(namespace: &str, sql: &str) -> Result<(String, Access)> {
    if sql.contains("--") || sql.contains("/*") { return Err(ControlError::AccessDenied("comments are not accepted".into())); }
    let dialect = GenericDialect {};
    let mut statements = Parser::parse_sql(&dialect, sql).map_err(|e| ControlError::InvalidInput(e.to_string()))?;
    if statements.len() != 1 { return Err(ControlError::AccessDenied("exactly one statement is required".into())); }
    let stmt = statements.pop().unwrap();
    let access = match &stmt {
        sqlparser::ast::Statement::Query(_) => Access::Read,
        sqlparser::ast::Statement::Insert(_) | sqlparser::ast::Statement::CreateTable(_)
        | sqlparser::ast::Statement::Drop { .. } | sqlparser::ast::Statement::Delete(_)
        | sqlparser::ast::Statement::Update { .. } => Access::Write,
        _ => return Err(ControlError::AccessDenied("statement type is not allowed".into())),
    };
    let rewritten = qualify_tables(namespace, sql)?;
    Ok((rewritten, access))
}

/// Scans for FROM, JOIN, INTO, and TABLE keywords and qualifies the name
/// that follows each one. Names are plain identifiers only.
fn qualify_tables(namespace: &str, sql: &str) -> Result<String> {
    let bytes = sql.as_bytes();
    let mut out = String::with_capacity(sql.len() + 32);
    let mut i = 0usize;
    let mut found = false;
    while i < bytes.len() {
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') { i += 1; }
            let word = &sql[start..i];
            out.push_str(word);
            if matches!(word.to_ascii_uppercase().as_str(), "FROM" | "JOIN" | "INTO" | "TABLE") {
                let mut j = i;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() { j += 1; }
                let nstart = j;
                while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_' || bytes[j] == b'.') { j += 1; }
                let name = &sql[nstart..j];
                if !name.is_empty() {
                    found = true;
                    out.push(' ');
                    out.push_str(&qualify_name(namespace, name)?);
                    i = j;
                }
            }
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    if !found && !sql.to_ascii_uppercase().contains("SELECT") {
        return Err(ControlError::AccessDenied("no table reference found".into()));
    }
    Ok(out)
}

fn qualify_name(namespace: &str, name: &str) -> Result<String> {
    let parts: Vec<&str> = name.split('.').collect();
    let table = match parts.as_slice() {
        [t] => *t,
        [ns, t] if *ns == namespace => *t,
        [cat, ns, t] if *cat == "ice" && *ns == namespace => *t,
        _ => return Err(ControlError::AccessDenied(format!("table {name} is outside the tenant namespace"))),
    };
    if !valid_ident(table) { return Err(ControlError::AccessDenied(format!("bad table name {table}"))); }
    Ok(format!("ice.{namespace}.{table}"))
}

pub struct ControlPlane {
    store: ControlStore,
    engine: Arc<Mutex<EngineService>>,
    storage: Arc<StorageCatalog>,
}

impl ControlPlane {
    pub fn new(store: ControlStore, storage: Arc<StorageCatalog>) -> Self {
        Self { store, engine: Arc::new(Mutex::new(EngineService::new())), storage }
    }

    pub fn store(&self) -> &ControlStore { &self.store }
    pub async fn is_registered(&self, tenant_id: &str) -> bool { self.engine.lock().await.is_registered(tenant_id) }

    async fn register_engine_tenant(&self, tenant: &TenantRecord) -> Result<()> {
        let mut engine = self.engine.lock().await;
        if engine.is_registered(&tenant.id) { return Ok(()); }
        engine.register_tenant(tenant.id.clone(), ResourceGroup {
            cost_cap_units: tenant.cost_cap_units, ..ResourceGroup::default()
        })?;
        let provider = IcebergCatalogProvider::try_new(self.storage.catalog.clone()).await
            .map_err(|e| ControlError::Storage(e.into()))?;
        engine.context(&tenant.id)
            .map_err(ControlError::from)?
            .register_catalog("ice", Arc::new(provider));
        Ok(())
    }

    async fn unregister_engine_tenant(&self, tenant_id: &str) {
        let mut engine = self.engine.lock().await;
        if engine.is_registered(tenant_id) { let _ = engine.unregister_tenant(tenant_id); }
    }

    /// One call provisions a tenant, its first admin user, and a first
    /// scoped token. Records are inserted in one transaction. The storage
    /// namespace and engine registration follow; on failure the tenant is
    /// marked deleted so nothing half-provisioned can authenticate.
    pub async fn provision_tenant(
        &self, name: &str, admin_login: &str, admin_password: &str,
        cost_cap_units: Option<u64>, token_ttl_seconds: Option<i64>,
    ) -> Result<ProvisionedTenant> {
        if name.trim().is_empty() || admin_login.trim().is_empty() || admin_password.is_empty() {
            return Err(ControlError::InvalidInput("name, login, and password are required".into()));
        }
        let tenant_id = uuid::Uuid::now_v7().to_string();
        let user_id = uuid::Uuid::now_v7().to_string();
        let token_id = uuid::Uuid::now_v7().to_string();
        let namespace = namespace_for_tenant(&tenant_id);
        let plaintext = credentials::generate_token();
        let mut tx = self.store.pool().begin().await?;
        let tres = async {
            store::insert_tenant(&mut tx, &NewTenant {
                id: tenant_id.clone(), name: name.to_string(), cost_cap_units, namespace: namespace.clone(),
            }).await?;
            store::insert_user(&mut tx, &NewUser {
                id: user_id.clone(), tenant_id: tenant_id.clone(), login: admin_login.to_string(),
                credential_hash: credentials::hash_password(admin_password), role: Role::Admin,
            }).await?;
            store::insert_token(&mut tx, &NewToken {
                id: token_id.clone(), tenant_id: tenant_id.clone(), user_id: user_id.clone(),
                token_hash: credentials::hash_token(&plaintext),
                scopes: vec![TokenScope::Write],
                ttl_seconds: token_ttl_seconds.unwrap_or(DEFAULT_TOKEN_TTL_SECONDS),
            }).await
        }.await;
        if let Err(e) = tres {
            let _ = tx.rollback().await;
            // The tenant name unique constraint is the expected duplicate.
            // A user or token clash inside a fresh tenant id cannot occur.
            if let ControlError::Database(d) = &e {
                if crate::types::is_unique_violation(d) { return Err(ControlError::DuplicateTenant); }
            }
            return Err(e);
        }
        tx.commit().await?;
        let tenant = self.store.get_tenant_by_id(&tenant_id).await?
            .ok_or_else(|| ControlError::InvalidState("tenant missing after provision".into()))?;
        let admin_user = self.store.get_user_by_id(&tenant_id, &user_id).await?
            .ok_or_else(|| ControlError::InvalidState("admin missing after provision".into()))?;
        let token_record = self.store.get_token_record(&tenant_id, &token_id).await?
            .ok_or_else(|| ControlError::InvalidState("token missing after provision".into()))?;
        let ready = async {
            self.storage.create_namespace(&NamespaceIdent::new(namespace.clone()))
                .await.map_err(|e| ControlError::Storage(e))?;
            self.register_engine_tenant(&tenant).await
        }.await;
        if let Err(e) = ready {
            let _ = self.store.update_tenant_status(&tenant_id, TenantStatus::Deleted).await;
            self.unregister_engine_tenant(&tenant_id).await;
            return Err(e);
        }
        Ok(ProvisionedTenant { tenant, admin_user, token: plaintext, token_record })
    }

    pub async fn create_user(&self, tenant_id: &str, login: &str, password: &str, role: Role) -> Result<crate::types::UserRecord> {
        let tenant = self.store.get_tenant_by_id(tenant_id).await?.ok_or(ControlError::NotFound)?;
        if tenant.status != TenantStatus::Active { return Err(ControlError::InvalidState("tenant is not active".into())); }
        if login.trim().is_empty() || password.is_empty() { return Err(ControlError::InvalidInput("login and password are required".into())); }
        let user_id = uuid::Uuid::now_v7().to_string();
        let mut tx = self.store.pool().begin().await?;
        let res = store::insert_user(&mut tx, &NewUser {
            id: user_id.clone(), tenant_id: tenant_id.to_string(), login: login.to_string(),
            credential_hash: credentials::hash_password(password), role,
        }).await;
        if let Err(e) = res {
            let _ = tx.rollback().await;
            if matches!(&e, ControlError::Database(d) if crate::types::is_unique_violation(d)) {
                return Err(ControlError::DuplicateUser);
            }
            return Err(e);
        }
        tx.commit().await?;
        self.store.get_user_by_id(tenant_id, &user_id).await?.ok_or(ControlError::NotFound)
    }

    pub async fn issue_token(&self, tenant_id: &str, user_id: &str, scopes: Vec<TokenScope>, ttl_seconds: Option<i64>) -> Result<IssuedToken> {
        let tenant = self.store.get_tenant_by_id(tenant_id).await?.ok_or(ControlError::NotFound)?;
        if tenant.status != TenantStatus::Active { return Err(ControlError::InvalidState("tenant is not active".into())); }
        if self.store.get_user_by_id(tenant_id, user_id).await?.is_none() { return Err(ControlError::NotFound); }
        let scopes = if scopes.is_empty() { vec![TokenScope::Read] } else { scopes };
        let token_id = uuid::Uuid::now_v7().to_string();
        let plaintext = credentials::generate_token();
        let mut tx = self.store.pool().begin().await?;
        store::insert_token(&mut tx, &NewToken {
            id: token_id.clone(), tenant_id: tenant_id.to_string(), user_id: user_id.to_string(),
            token_hash: credentials::hash_token(&plaintext), scopes,
            ttl_seconds: ttl_seconds.unwrap_or(DEFAULT_TOKEN_TTL_SECONDS),
        }).await?;
        tx.commit().await?;
        let record = self.store.get_token_record(tenant_id, &token_id).await?.ok_or(ControlError::NotFound)?;
        Ok(IssuedToken { record, token: plaintext })
    }

    pub async fn revoke_token(&self, tenant_id: &str, token_id: &str) -> Result<()> {
        self.store.revoke_token(tenant_id, token_id).await
    }

    /// The shared auth check. Token, expiry, revocation, tenant status, and
    /// user status are checked on every call. Every failure is AuthFailed.
    pub async fn authenticate_token(&self, plaintext: &str) -> Result<Principal> {
        if plaintext.is_empty() { return Err(ControlError::AuthFailed); }
        let hash = credentials::hash_token(plaintext);
        let row = sqlx::query(
            "SELECT t.tenant_id, t.user_id, t.scopes, tn.name AS tenant_name, tn.namespace, tn.status AS tenant_status, u.login, u.role, u.status AS user_status \
             FROM tokens t JOIN tenants tn ON tn.id = t.tenant_id JOIN users u ON u.id = t.user_id AND u.tenant_id = t.tenant_id \
             WHERE t.token_hash = $1 AND t.revoked = false AND t.expires_at > now()",
        ).bind(&hash).fetch_optional(self.store.pool()).await.map_err(|_| ControlError::AuthFailed)?;
        let row = row.ok_or(ControlError::AuthFailed)?;
        use sqlx::Row;
        let tenant_status: String = row.try_get("tenant_status").map_err(|_| ControlError::AuthFailed)?;
        let user_status: String = row.try_get("user_status").map_err(|_| ControlError::AuthFailed)?;
        if tenant_status != "active" || user_status != "active" { return Err(ControlError::AuthFailed); }
        let scopes_raw: String = row.try_get("scopes").map_err(|_| ControlError::AuthFailed)?;
        let scopes = crate::types::scopes_from_string(&scopes_raw).ok_or(ControlError::AuthFailed)?;
        let role_raw: String = row.try_get("role").map_err(|_| ControlError::AuthFailed)?;
        Ok(Principal {
            tenant_id: row.try_get("tenant_id").map_err(|_| ControlError::AuthFailed)?,
            tenant_name: row.try_get("tenant_name").map_err(|_| ControlError::AuthFailed)?,
            namespace: row.try_get("namespace").map_err(|_| ControlError::AuthFailed)?,
            user_id: row.try_get("user_id").map_err(|_| ControlError::AuthFailed)?,
            login: row.try_get("login").map_err(|_| ControlError::AuthFailed)?,
            role: Role::parse(&role_raw).ok_or(ControlError::AuthFailed)?,
            scopes,
        })
    }

    /// Password auth scoped to one tenant by tenant name. A missing login
    /// still pays one verification against a dummy hash.
    pub async fn authenticate_password(&self, tenant_name: &str, login: &str, password: &str) -> Result<Principal> {
        let tenant = self.store.get_tenant_by_name(tenant_name).await.map_err(|_| ControlError::AuthFailed)?
            .ok_or(ControlError::AuthFailed)?;
        if tenant.status != TenantStatus::Active { return Err(ControlError::AuthFailed); }
        let found = self.store.get_user_with_hash(&tenant.id, login).await.map_err(|_| ControlError::AuthFailed)?;
        let (user, stored) = match found {
            Some((u, h)) if u.status == "active" => (u, h),
            Some((u, _)) => { let _ = credentials::verify_password(password, &credentials::dummy_password_hash()); let _ = u; return Err(ControlError::AuthFailed); }
            None => { let _ = credentials::verify_password(password, &credentials::dummy_password_hash()); return Err(ControlError::AuthFailed); }
        };
        if !credentials::verify_password(password, &stored) { return Err(ControlError::AuthFailed); }
        Ok(Principal {
            tenant_id: tenant.id.clone(), tenant_name: tenant.name.clone(), namespace: tenant.namespace.clone(),
            user_id: user.id.clone(), login: user.login.clone(), role: user.role,
            scopes: vec![TokenScope::Read],
        })
    }

    /// Auth, status, scope, namespace, and cost cap checks all happen before
    /// EngineService::execute runs the SQL. Write scope implies read.
    pub async fn execute_authenticated(&self, plaintext_token: &str, sql: &str) -> Result<QueryResult> {
        let principal = self.authenticate_token(plaintext_token).await?;
        let tenant = self.store.get_tenant_by_id(&principal.tenant_id).await.map_err(|_| ControlError::AuthFailed)?
            .ok_or(ControlError::AuthFailed)?;
        if tenant.status != TenantStatus::Active || tenant.namespace != principal.namespace { return Err(ControlError::AuthFailed); }
        let (rewritten, access) = guard_sql(&principal.namespace, sql)?;
        let allowed = match access {
            Access::Read => principal.has_scope(TokenScope::Read) || principal.has_scope(TokenScope::Write),
            Access::Write => principal.has_scope(TokenScope::Write),
        };
        if !allowed { return Err(ControlError::AccessDenied("token scope does not allow this statement".into())); }
        let engine = self.engine.lock().await;
        Ok(engine.execute(&principal.tenant_id, &rewritten).await?)
    }

    /// Table names visible to this principal. The catalog query is scoped
    /// to the principal's own namespace, so no other tenant's tables list.
    pub async fn list_tables(&self, plaintext_token: &str) -> Result<Vec<String>> {
        let principal = self.authenticate_token(plaintext_token).await?;
        let namespace = NamespaceIdent::new(principal.namespace.clone());
        let idents = self.storage.catalog.list_tables(&namespace).await
            .map_err(|e| ControlError::Storage(e.into()))?;
        let mut names: Vec<String> = idents.iter().map(|t| t.name().to_string()).collect();
        names.sort();
        Ok(names)
    }

    pub async fn suspend_tenant(&self, tenant_id: &str) -> Result<TenantRecord> {
        let tenant = self.store.get_tenant_by_id(tenant_id).await?.ok_or(ControlError::NotFound)?;
        if tenant.status == TenantStatus::Deleted { return Err(ControlError::InvalidState("tenant is deleted".into())); }
        self.store.update_tenant_status(tenant_id, TenantStatus::Suspended).await?;
        self.unregister_engine_tenant(tenant_id).await;
        self.store.get_tenant_by_id(tenant_id).await?.ok_or(ControlError::NotFound)
    }

    pub async fn unsuspend_tenant(&self, tenant_id: &str) -> Result<TenantRecord> {
        let tenant = self.store.get_tenant_by_id(tenant_id).await?.ok_or(ControlError::NotFound)?;
        if tenant.status != TenantStatus::Suspended { return Err(ControlError::InvalidState("tenant is not suspended".into())); }
        self.store.update_tenant_status(tenant_id, TenantStatus::Active).await?;
        let tenant = self.store.get_tenant_by_id(tenant_id).await?.ok_or(ControlError::NotFound)?;
        self.register_engine_tenant(&tenant).await?;
        Ok(tenant)
    }

    /// Marks the tenant deleted, revokes its tokens, and unregisters it
    /// from the engine. Control rows stay as a deleted tombstone. Storage
    /// tables and snapshots are not erased in this slice.
    pub async fn delete_tenant(&self, tenant_id: &str) -> Result<TenantRecord> {
        let tenant = self.store.get_tenant_by_id(tenant_id).await?.ok_or(ControlError::NotFound)?;
        if tenant.status == TenantStatus::Deleted { return Err(ControlError::InvalidState("tenant is already deleted".into())); }
        let mut tx = self.store.pool().begin().await?;
        sqlx::query("UPDATE tokens SET revoked = true, updated_at = now() WHERE tenant_id = $1")
            .bind(tenant_id).execute(&mut *tx).await?;
        sqlx::query("UPDATE tenants SET status = 'deleted', updated_at = now() WHERE id = $1")
            .bind(tenant_id).execute(&mut *tx).await?;
        tx.commit().await?;
        self.unregister_engine_tenant(tenant_id).await;
        self.store.get_tenant_by_id(tenant_id).await?.ok_or(ControlError::NotFound)
    }

    pub async fn update_cost_cap(&self, tenant_id: &str, cap: Option<u64>) -> Result<TenantRecord> {
        let tenant = self.store.get_tenant_by_id(tenant_id).await?.ok_or(ControlError::NotFound)?;
        if tenant.status == TenantStatus::Deleted { return Err(ControlError::InvalidState("tenant is deleted".into())); }
        self.store.update_cost_cap(tenant_id, cap).await?;
        let tenant = self.store.get_tenant_by_id(tenant_id).await?.ok_or(ControlError::NotFound)?;
        if tenant.status == TenantStatus::Active {
            self.unregister_engine_tenant(tenant_id).await;
            self.register_engine_tenant(&tenant).await?;
        }
        Ok(tenant)
    }

    pub fn default_resource_group(cost_cap_units: Option<u64>) -> ResourceGroup {
        ResourceGroup { cost_cap_units, ..ResourceGroup::default() }
    }

    pub fn token_ttl_default() -> Duration { Duration::from_secs(DEFAULT_TOKEN_TTL_SECONDS as u64) }
}
