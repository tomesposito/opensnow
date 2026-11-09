//! Postgres control plane store. Every tenant-scoped read filters by
//! tenant id in the query itself. Schema setup is explicit and idempotent:
//! plain SQL, no migration framework.
use sqlx::{PgPool, Row};
use crate::types::{scopes_from_string, scopes_to_string, ControlError, Result, Role, TenantRecord, TenantStatus, TokenRecord, TokenScope, UserRecord};

pub const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS tenants (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL,
    cost_cap_units BIGINT,
    namespace TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    login TEXT NOT NULL,
    credential_hash TEXT NOT NULL,
    role TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, login)
);
CREATE TABLE IF NOT EXISTS tokens (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    user_id TEXT NOT NULL REFERENCES users(id),
    token_hash TEXT NOT NULL UNIQUE,
    scopes TEXT NOT NULL,
    revoked BOOLEAN NOT NULL DEFAULT false,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS tokens_tenant_idx ON tokens (tenant_id);
CREATE INDEX IF NOT EXISTS users_tenant_idx ON users (tenant_id);
"#;

fn tenant_from_row(row: &sqlx::postgres::PgRow) -> Result<TenantRecord> {
    let status_raw: String = row.try_get("status")?;
    let status = TenantStatus::parse(&status_raw)
        .ok_or_else(|| ControlError::InvalidState(format!("unknown tenant status {status_raw}")))?;
    let cap: Option<i64> = row.try_get("cost_cap_units")?;
    let cost_cap_units = match cap {
        None => None,
        Some(v) => Some(u64::try_from(v).map_err(|_| ControlError::InvalidState("negative cost cap".into()))?),
    };
    Ok(TenantRecord {
        id: row.try_get("id")?, name: row.try_get("name")?, status, cost_cap_units,
        namespace: row.try_get("namespace")?,
        created_at: row.try_get("created_at")?, updated_at: row.try_get("updated_at")?,
    })
}

fn user_from_row(row: &sqlx::postgres::PgRow) -> Result<UserRecord> {
    let role_raw: String = row.try_get("role")?;
    let role = Role::parse(&role_raw)
        .ok_or_else(|| ControlError::InvalidState(format!("unknown role {role_raw}")))?;
    Ok(UserRecord {
        id: row.try_get("id")?, tenant_id: row.try_get("tenant_id")?,
        login: row.try_get("login")?, role, status: row.try_get("status")?,
        created_at: row.try_get("created_at")?, updated_at: row.try_get("updated_at")?,
    })
}

fn token_from_row(row: &sqlx::postgres::PgRow) -> Result<TokenRecord> {
    let scopes_raw: String = row.try_get("scopes")?;
    let scopes = scopes_from_string(&scopes_raw)
        .ok_or_else(|| ControlError::InvalidState("unknown token scope".into()))?;
    Ok(TokenRecord {
        id: row.try_get("id")?, tenant_id: row.try_get("tenant_id")?,
        user_id: row.try_get("user_id")?, scopes, revoked: row.try_get("revoked")?,
        expires_at: row.try_get("expires_at")?,
        created_at: row.try_get("created_at")?, updated_at: row.try_get("updated_at")?,
    })
}

const TENANT_COLS: &str = "id, name, status, cost_cap_units, namespace, created_at::text AS created_at, updated_at::text AS updated_at";
const USER_COLS: &str = "id, tenant_id, login, role, status, created_at::text AS created_at, updated_at::text AS updated_at";
const TOKEN_COLS: &str = "id, tenant_id, user_id, scopes, revoked, expires_at::text AS expires_at, created_at::text AS created_at, updated_at::text AS updated_at";

#[derive(Clone)]
pub struct ControlStore { pool: PgPool }

impl ControlStore {
    pub fn new(pool: PgPool) -> Self { Self { pool } }
    pub fn pool(&self) -> &PgPool { &self.pool }

    pub async fn ensure_schema(&self) -> Result<()> {
        // raw_sql uses the simple query protocol: SCHEMA_SQL holds several
        // CREATE statements, and a prepared statement accepts only one.
        sqlx::raw_sql(SCHEMA_SQL).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn get_tenant_by_id(&self, tenant_id: &str) -> Result<Option<TenantRecord>> {
        let row = sqlx::query(&format!("SELECT {TENANT_COLS} FROM tenants WHERE id = $1"))
            .bind(tenant_id).fetch_optional(&self.pool).await?;
        row.as_ref().map(tenant_from_row).transpose()
    }

    pub async fn get_tenant_by_name(&self, name: &str) -> Result<Option<TenantRecord>> {
        let row = sqlx::query(&format!("SELECT {TENANT_COLS} FROM tenants WHERE name = $1"))
            .bind(name).fetch_optional(&self.pool).await?;
        row.as_ref().map(tenant_from_row).transpose()
    }

    pub async fn get_user_by_login(&self, tenant_id: &str, login: &str) -> Result<Option<UserRecord>> {
        let row = sqlx::query(&format!("SELECT {USER_COLS} FROM users WHERE tenant_id = $1 AND login = $2"))
            .bind(tenant_id).bind(login).fetch_optional(&self.pool).await?;
        row.as_ref().map(user_from_row).transpose()
    }

    pub async fn get_user_by_id(&self, tenant_id: &str, user_id: &str) -> Result<Option<UserRecord>> {
        let row = sqlx::query(&format!("SELECT {USER_COLS} FROM users WHERE tenant_id = $1 AND id = $2"))
            .bind(tenant_id).bind(user_id).fetch_optional(&self.pool).await?;
        row.as_ref().map(user_from_row).transpose()
    }

    /// Credential lookup for password auth. Returns the stored hash with the
    /// user row. The hash never leaves the crate's auth path.
    pub async fn get_user_with_hash(&self, tenant_id: &str, login: &str) -> Result<Option<(UserRecord, String)>> {
        let row = sqlx::query(&format!("SELECT {USER_COLS}, credential_hash FROM users WHERE tenant_id = $1 AND login = $2"))
            .bind(tenant_id).bind(login).fetch_optional(&self.pool).await?;
        match row {
            None => Ok(None),
            Some(r) => Ok(Some((user_from_row(&r)?, r.try_get("credential_hash")?))),
        }
    }

    pub async fn get_token_record(&self, tenant_id: &str, token_id: &str) -> Result<Option<TokenRecord>> {
        let row = sqlx::query(&format!("SELECT {TOKEN_COLS} FROM tokens WHERE tenant_id = $1 AND id = $2"))
            .bind(tenant_id).bind(token_id).fetch_optional(&self.pool).await?;
        row.as_ref().map(token_from_row).transpose()
    }

    /// Stored hash for a token, for tests that prove plaintext is not stored.
    pub async fn stored_token_hash(&self, tenant_id: &str, token_id: &str) -> Result<Option<String>> {
        let row = sqlx::query("SELECT token_hash FROM tokens WHERE tenant_id = $1 AND id = $2")
            .bind(tenant_id).bind(token_id).fetch_optional(&self.pool).await?;
        match row { None => Ok(None), Some(r) => Ok(Some(r.try_get("token_hash")?)) }
    }

    pub async fn update_tenant_status(&self, tenant_id: &str, status: TenantStatus) -> Result<()> {
        let done = sqlx::query("UPDATE tenants SET status = $2, updated_at = now() WHERE id = $1")
            .bind(tenant_id).bind(status.as_str()).execute(&self.pool).await?;
        if done.rows_affected() != 1 { return Err(ControlError::NotFound); }
        Ok(())
    }

    pub async fn update_cost_cap(&self, tenant_id: &str, cap: Option<u64>) -> Result<()> {
        let cap_i64: Option<i64> = match cap {
            None => None,
            Some(v) => Some(i64::try_from(v).map_err(|_| ControlError::InvalidInput("cost cap too large".into()))?),
        };
        let done = sqlx::query("UPDATE tenants SET cost_cap_units = $2, updated_at = now() WHERE id = $1")
            .bind(tenant_id).bind(cap_i64).execute(&self.pool).await?;
        if done.rows_affected() != 1 { return Err(ControlError::NotFound); }
        Ok(())
    }

    pub async fn revoke_token(&self, tenant_id: &str, token_id: &str) -> Result<()> {
        let done = sqlx::query("UPDATE tokens SET revoked = true, updated_at = now() WHERE tenant_id = $1 AND id = $2")
            .bind(tenant_id).bind(token_id).execute(&self.pool).await?;
        if done.rows_affected() != 1 { return Err(ControlError::NotFound); }
        Ok(())
    }
}

/// Inputs for the single-transaction provisioning inserts.
pub struct NewTenant { pub id: String, pub name: String, pub cost_cap_units: Option<u64>, pub namespace: String }
pub struct NewUser { pub id: String, pub tenant_id: String, pub login: String, pub credential_hash: String, pub role: Role }
pub struct NewToken { pub id: String, pub tenant_id: String, pub user_id: String, pub token_hash: String, pub scopes: Vec<TokenScope>, pub ttl_seconds: i64 }

pub async fn insert_tenant(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, t: &NewTenant) -> Result<()> {
    let cap: Option<i64> = match t.cost_cap_units {
        None => None,
        Some(v) => Some(i64::try_from(v).map_err(|_| ControlError::InvalidInput("cost cap too large".into()))?),
    };
    sqlx::query("INSERT INTO tenants (id, name, status, cost_cap_units, namespace) VALUES ($1, $2, 'active', $3, $4)")
        .bind(&t.id).bind(&t.name).bind(cap).bind(&t.namespace).execute(&mut **tx).await?;
    Ok(())
}

pub async fn insert_user(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, u: &NewUser) -> Result<()> {
    sqlx::query("INSERT INTO users (id, tenant_id, login, credential_hash, role, status) VALUES ($1, $2, $3, $4, $5, 'active')")
        .bind(&u.id).bind(&u.tenant_id).bind(&u.login).bind(&u.credential_hash).bind(u.role.as_str())
        .execute(&mut **tx).await?;
    Ok(())
}

pub async fn insert_token(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, t: &NewToken) -> Result<()> {
    sqlx::query("INSERT INTO tokens (id, tenant_id, user_id, token_hash, scopes, expires_at) VALUES ($1, $2, $3, $4, $5, now() + make_interval(secs => $6))")
        .bind(&t.id).bind(&t.tenant_id).bind(&t.user_id).bind(&t.token_hash)
        .bind(scopes_to_string(&t.scopes)).bind(t.ttl_seconds as f64)
        .execute(&mut **tx).await?;
    Ok(())
}
