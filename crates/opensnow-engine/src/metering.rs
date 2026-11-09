//! Per-query cost metering. See docs/METERING.md.
//!
//! What is measured: elapsed wall time, rows in the batches actually
//! pulled from the result stream, and the in-memory size of those
//! batches. What is modeled: allocated vCPU-seconds (elapsed times the
//! tenant's target partitions) priced at the internal cost rate from
//! docs/COST-MODEL.md. The dollars are cost attribution, not a price.
use std::time::Duration;

/// Internal cost basis from docs/COST-MODEL.md Section 1: the EC2 m7i
/// family prices linearly at $0.0504 per vCPU-hour, us-east-1 on-demand.
pub const DEFAULT_VCPU_HOUR_DOLLARS: f64 = 0.0504;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeteringRates {
    pub vcpu_hour_dollars: f64,
}
impl Default for MeteringRates {
    fn default() -> Self { Self { vcpu_hour_dollars: DEFAULT_VCPU_HOUR_DOLLARS } }
}
impl MeteringRates {
    /// Pure dollar math, kept separate so it can be tested exactly.
    pub fn cost_dollars(&self, vcpu_seconds: f64) -> f64 {
        vcpu_seconds / 3600.0 * self.vcpu_hour_dollars
    }
}

/// Pure form of the same math, for callers holding a bare rate.
pub fn cost_dollars(vcpu_seconds: f64, vcpu_hour_dollars: f64) -> f64 {
    MeteringRates { vcpu_hour_dollars }.cost_dollars(vcpu_seconds)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CostRecordKind { Query, Background }
impl CostRecordKind {
    pub fn as_str(&self) -> &'static str { match self { Self::Query => "query", Self::Background => "background" } }
    pub fn parse(s: &str) -> Option<Self> { match s { "query" => Some(Self::Query), "background" => Some(Self::Background), _ => None } }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CostRecordStatus { Completed, Failed, CostCapExceeded }
impl CostRecordStatus {
    pub fn as_str(&self) -> &'static str {
        match self { Self::Completed => "completed", Self::Failed => "failed", Self::CostCapExceeded => "cost_cap_exceeded" }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s { "completed" => Some(Self::Completed), "failed" => Some(Self::Failed), "cost_cap_exceeded" => Some(Self::CostCapExceeded), _ => None }
    }
}

/// One metered unit of work. `rows_returned` and `bytes_returned` count
/// the batches actually pulled from the stream: an aborting batch that
/// the existing accounting charges counts, a batch never pulled does
/// not. `bytes_returned` is the in-memory size of those batches (bytes
/// produced), never bytes scanned from storage.
#[derive(Clone, Debug, PartialEq)]
pub struct CostRecord {
    pub tenant: String,
    pub seq: u64,
    pub kind: CostRecordKind,
    pub status: CostRecordStatus,
    pub sql: Option<String>,
    pub service: Option<String>,
    pub started_at_unix_ms: u64,
    pub elapsed: Duration,
    pub rows_returned: u64,
    pub bytes_returned: u64,
    pub vcpu_seconds: f64,
    pub cost_dollars: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TenantCostSummary {
    pub record_count: u64,
    pub total_cost_dollars: f64,
    pub total_vcpu_seconds: f64,
    pub total_elapsed: Duration,
    pub total_rows_returned: u64,
    pub total_bytes_returned: u64,
}
impl TenantCostSummary {
    pub fn from_records(records: &[CostRecord]) -> Self {
        let mut summary = Self::default();
        for record in records { summary.add(record); }
        summary
    }
    pub fn add(&mut self, record: &CostRecord) {
        self.record_count += 1;
        self.total_cost_dollars += record.cost_dollars;
        self.total_vcpu_seconds += record.vcpu_seconds;
        self.total_elapsed += record.elapsed;
        self.total_rows_returned += record.rows_returned;
        self.total_bytes_returned += record.bytes_returned;
    }
}
