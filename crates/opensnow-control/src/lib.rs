//! opensnow control plane: tenants, users, tokens, and authenticated
//! execution. See docs/CONTROL-PLANE.md and the issue #9 brief.

pub mod credentials;
pub mod service;
pub mod store;
pub mod types;

pub use service::{namespace_for_tenant, ControlPlane};
pub use store::ControlStore;
pub use opensnow_engine::{CostRecord, CostRecordKind, CostRecordStatus, MeteringRates, TenantCostSummary};
pub use types::{
    ControlError, IssuedToken, Principal, ProvisionedTenant, Result, Role,
    TenantRecord, TenantStatus, TokenRecord, TokenScope, UserRecord,
};
