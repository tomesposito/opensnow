use std::fmt;
use opensnow_engine::EngineError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TenantStatus { Active, Suspended, Deleted }
impl TenantStatus {
    pub fn as_str(&self) -> &'static str { match self { Self::Active => "active", Self::Suspended => "suspended", Self::Deleted => "deleted" } }
    pub fn parse(s: &str) -> Option<Self> { match s { "active" => Some(Self::Active), "suspended" => Some(Self::Suspended), "deleted" => Some(Self::Deleted), _ => None } }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role { Admin, Member }
impl Role {
    pub fn as_str(&self) -> &'static str { match self { Self::Admin => "admin", Self::Member => "member" } }
    pub fn parse(s: &str) -> Option<Self> { match s { "admin" => Some(Self::Admin), "member" => Some(Self::Member), _ => None } }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenScope { Read, Write }
impl TokenScope {
    pub fn as_str(&self) -> &'static str { match self { Self::Read => "read", Self::Write => "write" } }
    pub fn parse(s: &str) -> Option<Self> { match s.trim() { "read" => Some(Self::Read), "write" => Some(Self::Write), _ => None } }
}
pub fn scopes_to_string(scopes: &[TokenScope]) -> String { scopes.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(",") }
pub fn scopes_from_string(s: &str) -> Option<Vec<TokenScope>> { s.split(',').map(TokenScope::parse).collect() }

#[derive(Clone, Debug)]
pub struct TenantRecord {
    pub id: String, pub name: String, pub status: TenantStatus,
    pub cost_cap_units: Option<u64>, pub namespace: String,
    pub created_at: String, pub updated_at: String,
}
#[derive(Clone, Debug)]
pub struct UserRecord {
    pub id: String, pub tenant_id: String, pub login: String, pub role: Role,
    pub status: String, pub created_at: String, pub updated_at: String,
}
#[derive(Clone, Debug)]
pub struct TableColumn { pub name: String, pub data_type: String }
#[derive(Clone, Debug)]
pub struct TokenRecord {
    pub id: String, pub tenant_id: String, pub user_id: String, pub scopes: Vec<TokenScope>,
    pub revoked: bool, pub expires_at: String, pub created_at: String, pub updated_at: String,
}
/// Returned exactly once at provisioning. `token` is plaintext and is never stored.
#[derive(Clone, Debug)]
pub struct ProvisionedTenant { pub tenant: TenantRecord, pub admin_user: UserRecord, pub token: String, pub token_record: TokenRecord }
#[derive(Clone, Debug)]
pub struct IssuedToken { pub record: TokenRecord, pub token: String }
#[derive(Clone, Debug)]
pub struct Principal {
    pub tenant_id: String, pub tenant_name: String, pub namespace: String,
    pub user_id: String, pub login: String, pub role: Role, pub scopes: Vec<TokenScope>,
}
impl Principal { pub fn has_scope(&self, scope: TokenScope) -> bool { self.scopes.contains(&scope) } }

/// Auth failures are deliberately opaque: one variant and one message for a
/// wrong credential, an expired or revoked token, and a suspended or deleted
/// tenant, so an unauthenticated caller cannot enumerate valid logins.
#[derive(Debug)]
pub enum ControlError {
    AuthFailed,
    AccessDenied(String),
    DuplicateTenant,
    DuplicateUser,
    NotFound,
    InvalidInput(String),
    InvalidState(String),
    CostCapExceeded,
    Engine(EngineError),
    Database(sqlx::Error),
    Storage(Box<dyn std::error::Error + Send + Sync>),
}
impl fmt::Display for ControlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AuthFailed => write!(f, "authentication failed"),
            Self::AccessDenied(m) => write!(f, "access denied: {m}"),
            Self::DuplicateTenant => write!(f, "tenant name already exists"),
            Self::DuplicateUser => write!(f, "user login already exists for tenant"),
            Self::NotFound => write!(f, "not found"),
            Self::InvalidInput(m) => write!(f, "invalid input: {m}"),
            Self::InvalidState(m) => write!(f, "invalid state: {m}"),
            Self::CostCapExceeded => write!(f, "cost cap exceeded"),
            Self::Engine(e) => write!(f, "engine error: {e}"),
            Self::Database(e) => write!(f, "database error: {e}"),
            Self::Storage(e) => write!(f, "storage error: {e}"),
        }
    }
}
impl std::error::Error for ControlError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self { Self::Engine(e) => Some(e), Self::Database(e) => Some(e), Self::Storage(e) => Some(e.as_ref()), _ => None }
    }
}
impl From<EngineError> for ControlError {
    fn from(e: EngineError) -> Self { match e { EngineError::CostCapExceeded => Self::CostCapExceeded, other => Self::Engine(other) } }
}
impl From<sqlx::Error> for ControlError { fn from(e: sqlx::Error) -> Self { Self::Database(e) } }
pub type Result<T> = std::result::Result<T, ControlError>;

pub fn is_unique_violation(e: &sqlx::Error) -> bool {
    matches!(e, sqlx::Error::Database(db) if db.code().as_deref() == Some("23505"))
}
