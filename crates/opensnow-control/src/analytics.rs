use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::Row;

use crate::store::{worksheet_from_row, WORKSHEET_COLS};
use crate::{ControlError, ControlPlane, Result, TokenScope, WorksheetRecord};

const V5_SCHEMA: &str = "https://vega.github.io/schema/vega-lite/v5.json";

/// Accept the unit charts this slice draws from worksheet results.
pub fn validate_chart(spec: &Value) -> Result<()> {
    let valid = spec.as_object().is_some_and(|object| {
        object.keys().all(|key| matches!(key.as_str(), "$schema" | "mark" | "encoding" | "title" | "data"))
            && spec.get("$schema").is_none_or(|schema| schema.as_str() == Some(V5_SCHEMA))
            && spec.get("title").is_none_or(Value::is_string)
            && spec.get("data").is_none_or(|data| {
                data.as_object().is_some_and(|source| source.len() == 1 && data["name"].is_string())
            })
            && valid_encoding(&spec["mark"], &spec["encoding"])
    });
    if !valid {
        return Err(ControlError::InvalidInput("expected a supported Vega-Lite v5 chart over worksheet results".into()));
    }
    Ok(())
}

fn valid_encoding(mark: &Value, encoding: &Value) -> bool {
    let (required, allowed): ([_; 2], &[_]) = match mark.as_str() {
        Some("arc") => (["theta", "color"], &["theta", "color", "detail"]),
        Some("bar") => (["x", "y"], &["x", "y", "color", "xOffset"]),
        Some("line" | "area" | "point" | "circle" | "rect") => (["x", "y"], &["x", "y", "color"]),
        _ => return false,
    };
    encoding.as_object().is_some_and(|channels| {
        required.iter().all(|key| channels.contains_key(*key))
            && channels.iter().all(|(name, field)| allowed.contains(&name.as_str()) && valid_field(field))
    })
}

fn valid_field(field: &Value) -> bool {
    field.as_object().is_some_and(|object| {
        object.keys().all(|key| matches!(key.as_str(), "field" | "type" | "aggregate" | "title"))
            && field.get("type").is_none_or(|ty| matches!(ty.as_str(), Some("nominal" | "ordinal" | "quantitative" | "temporal")))
            && field.get("aggregate").is_none_or(|agg| matches!(agg.as_str(), Some("sum" | "mean" | "average" | "count" | "min" | "max")))
            && field.get("title").is_none_or(Value::is_string)
            && (field["field"].as_str().is_some_and(|name| !name.is_empty())
                || (field.get("field").is_none() && field["aggregate"] == "count"))
    })
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Dashboard {
    pub id: String,
    pub name: String,
    pub worksheet_ids: Vec<String>,
    pub updated_at: String,
    pub last_refresh: Option<Value>,
}

pub struct DashboardTile {
    pub worksheet: WorksheetRecord,
    pub latest_result: Option<Value>,
}

pub fn dashboard_name(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ControlError::InvalidInput("dashboard name is required".into()));
    }
    Ok(name)
}

impl ControlPlane {
    pub async fn worksheet(&self, token: &str, id: &str) -> Result<WorksheetRecord> {
        let owner = self.authenticate_token(token).await?;
        self.store().get_worksheet(&owner.tenant_id, &owner.user_id, id).await?.ok_or(ControlError::NotFound)
    }

    pub async fn set_chart_spec(&self, token: &str, id: &str, spec: Option<Value>, require_write: bool) -> Result<()> {
        let owner = self.authenticate_token(token).await?;
        if require_write && !owner.has_scope(TokenScope::Write) {
            return Err(ControlError::AccessDenied("token scope does not allow chart edits".into()));
        }
        if let Some(spec) = &spec { validate_chart(spec)?; }
        let changed = sqlx::query(
            "UPDATE worksheets SET chart_spec = $4, updated_at = now() WHERE tenant_id = $1 AND user_id = $2 AND id = $3",
        ).bind(&owner.tenant_id).bind(&owner.user_id).bind(id).bind(spec)
            .execute(self.store().pool()).await?.rows_affected();
        if changed == 0 { return Err(ControlError::NotFound); }
        Ok(())
    }

    pub async fn list_dashboards(&self, token: &str) -> Result<Vec<Dashboard>> {
        let owner = self.authenticate_token(token).await?;
        let rows = sqlx::query_as::<_, (String, String, Vec<String>, String, Option<Value>)>(
            "SELECT id, name, worksheet_ids, updated_at::text, last_refresh FROM dashboards
             WHERE tenant_id = $1 AND user_id = $2 ORDER BY updated_at DESC, id",
        ).bind(&owner.tenant_id).bind(&owner.user_id).fetch_all(self.store().pool()).await?;
        Ok(rows.into_iter().map(|(id, name, worksheet_ids, updated_at, last_refresh)| Dashboard { id, name, worksheet_ids, updated_at, last_refresh }).collect())
    }

    pub async fn dashboard(&self, token: &str, id: &str) -> Result<(Dashboard, Vec<DashboardTile>)> {
        let owner = self.authenticate_token(token).await?;
        let mut tx = self.store().pool().begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY").execute(&mut *tx).await?;
        let (name, worksheet_ids, updated_at, last_refresh) = sqlx::query_as::<_, (String, Vec<String>, String, Option<Value>)>(
            "SELECT name, worksheet_ids, updated_at::text, last_refresh FROM dashboards WHERE tenant_id = $1 AND user_id = $2 AND id = $3",
        ).bind(&owner.tenant_id).bind(&owner.user_id).bind(id).fetch_optional(&mut *tx).await?.ok_or(ControlError::NotFound)?;
        let tiles = sqlx::query(&format!(
            "SELECT {WORKSHEET_COLS}, latest_tile_result FROM worksheets
             WHERE tenant_id = $1 AND user_id = $2 AND id = ANY($3) ORDER BY array_position($3, id)"
        )).bind(&owner.tenant_id).bind(&owner.user_id).bind(&worksheet_ids).fetch_all(&mut *tx).await?
            .iter().map(|row| Ok(DashboardTile { worksheet: worksheet_from_row(row)?, latest_result: row.try_get("latest_tile_result")? }))
            .collect::<Result<Vec<_>>>()?;
        tx.commit().await?;
        Ok((Dashboard { id: id.into(), name, worksheet_ids, updated_at, last_refresh }, tiles))
    }

    pub async fn save_dashboard(&self, token: &str, id: Option<&str>, name: &str, worksheet_ids: &[String]) -> Result<Dashboard> {
        let owner = self.authenticate_token(token).await?;
        let name = dashboard_name(name)?;
        let mut tx = self.store().pool().begin().await?;
        let worksheets = sqlx::query_scalar::<_, String>(
            "SELECT id FROM worksheets WHERE tenant_id = $1 AND user_id = $2 AND id = ANY($3) ORDER BY id FOR KEY SHARE",
        ).bind(&owner.tenant_id).bind(&owner.user_id).bind(worksheet_ids).fetch_all(&mut *tx).await?;
        if worksheets.len() != worksheet_ids.len() { return Err(ControlError::NotFound); }
        let (id, updated_at, last_refresh) = match id {
            Some(id) => sqlx::query_as::<_, (String, String, Option<Value>)>(
                "UPDATE dashboards SET name = $4, last_refresh = CASE WHEN worksheet_ids = $5 THEN last_refresh END,
                 worksheet_ids = $5, updated_at = now()
                 WHERE tenant_id = $1 AND user_id = $2 AND id = $3 RETURNING id, updated_at::text, last_refresh",
            ).bind(&owner.tenant_id).bind(&owner.user_id).bind(id).bind(name).bind(worksheet_ids)
                .fetch_optional(&mut *tx).await?.ok_or(ControlError::NotFound)?,
            None => sqlx::query_as::<_, (String, String, Option<Value>)>(
                "INSERT INTO dashboards (tenant_id, user_id, id, name, worksheet_ids)
                 VALUES ($1, $2, $3, $4, $5) RETURNING id, updated_at::text, last_refresh",
            ).bind(&owner.tenant_id).bind(&owner.user_id).bind(uuid::Uuid::now_v7().to_string()).bind(name).bind(worksheet_ids)
                .fetch_one(&mut *tx).await?,
        };
        tx.commit().await?;
        Ok(Dashboard { id, name: name.into(), worksheet_ids: worksheet_ids.to_vec(), updated_at, last_refresh })
    }

    pub async fn delete_dashboard(&self, token: &str, id: &str) -> Result<()> {
        let owner = self.authenticate_token(token).await?;
        let changed = sqlx::query("DELETE FROM dashboards WHERE tenant_id = $1 AND user_id = $2 AND id = $3")
            .bind(&owner.tenant_id).bind(&owner.user_id).bind(id).execute(self.store().pool()).await?.rows_affected();
        if changed == 0 { return Err(ControlError::NotFound); }
        Ok(())
    }

    pub async fn tile_worksheet(&self, token: &str, dashboard_id: &str, worksheet_id: &str) -> Result<WorksheetRecord> {
        let owner = self.authenticate_token(token).await?;
        let row = sqlx::query(&format!(
            "SELECT {WORKSHEET_COLS} FROM worksheets
             WHERE tenant_id = $1 AND user_id = $2 AND id = $4 AND EXISTS (
                 SELECT 1 FROM dashboards WHERE tenant_id = $1 AND user_id = $2 AND id = $3
                 AND $4 = ANY(worksheet_ids)
             )"
        )).bind(&owner.tenant_id).bind(&owner.user_id).bind(dashboard_id).bind(worksheet_id)
            .fetch_optional(self.store().pool()).await?.ok_or(ControlError::NotFound)?;
        worksheet_from_row(&row)
    }

    pub async fn save_dashboard_refresh(&self, token: &str, dashboard: &Dashboard, refresh: &Value) -> Result<()> {
        let owner = self.authenticate_token(token).await?;
        sqlx::query(
            "UPDATE dashboards SET last_refresh = $5 WHERE tenant_id = $1 AND user_id = $2 AND id = $3
             AND worksheet_ids = $4 AND COALESCE((last_refresh ->> 'refreshed_at')::bigint, 0)
                 <= ($5 ->> 'refreshed_at')::bigint",
        ).bind(&owner.tenant_id).bind(&owner.user_id).bind(&dashboard.id).bind(&dashboard.worksheet_ids)
            .bind(refresh).execute(self.store().pool()).await?;
        Ok(())
    }

    pub async fn save_tile_result(&self, token: &str, worksheet: &WorksheetRecord, result: &Value, seq: u64) -> Result<()> {
        let owner = self.authenticate_token(token).await?;
        sqlx::query(
            "UPDATE worksheets SET latest_tile_result = $5 WHERE tenant_id = $1 AND user_id = $2 AND id = $3
             AND sql_text = $4 AND COALESCE((latest_tile_result #>> '{outcome,cost_record_seq}')::bigint, 0) < $6",
        ).bind(&owner.tenant_id).bind(&owner.user_id).bind(&worksheet.id).bind(&worksheet.sql_text)
            .bind(result).bind(seq as i64).execute(self.store().pool()).await?;
        Ok(())
    }
}
