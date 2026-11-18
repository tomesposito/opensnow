use opensnow_control::analytics::{Dashboard, DashboardTile};
use serde_json::{json, Value};

use crate::local::LocalDb;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

pub(crate) fn worksheet_json(w: &opensnow_control::WorksheetRecord) -> Value {
    json!({
        "id": w.id, "name": w.name, "sql": w.sql_text, "chart_spec": w.chart_spec,
        "created_at": w.created_at, "updated_at": w.updated_at,
    })
}

pub(crate) fn dashboard_json(dashboard: Dashboard, tiles: Vec<DashboardTile>) -> Value {
    let mut data = json!(dashboard);
    data["tiles"] = json!(tiles.into_iter().map(|tile| {
        let mut view = worksheet_json(&tile.worksheet);
        view["latest_result"] = json!(tile.latest_result);
        view
    }).collect::<Vec<_>>());
    data
}

impl LocalDb {
    pub(crate) async fn chart_spec(&self, id: &str) -> Result<Option<Value>, BoxError> {
        let spec = sqlx::query_scalar::<_, Option<String>>("SELECT chart_spec FROM opensnow_worksheets WHERE id = ?")
            .bind(id).fetch_optional(&self.metadata).await?.ok_or("not found")?;
        Ok(spec.map(|s| serde_json::from_str(&s)).transpose()?)
    }

    pub(crate) async fn set_chart_spec(&self, id: &str, spec: Option<Value>) -> Result<(), BoxError> {
        if let Some(spec) = &spec { opensnow_control::analytics::validate_chart(spec)?; }
        let changed = sqlx::query(
            "UPDATE opensnow_worksheets SET chart_spec = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        ).bind(spec.map(|s| s.to_string())).bind(id).execute(&self.metadata).await?.rows_affected();
        if changed == 0 { return Err("not found".into()); }
        Ok(())
    }

    pub async fn save_worksheet(&self, id: Option<&str>, name: &str, sql: &str) -> Result<String, BoxError> {
        let name = name.trim();
        if name.is_empty() { return Err("worksheet name is required".into()); }
        match id {
            Some(id) => {
                let changed = sqlx::query("UPDATE opensnow_worksheets SET name = ?1, latest_tile_result = CASE WHEN sql_text = ?2 THEN latest_tile_result END,
                    sql_text = ?2, updated_at = CURRENT_TIMESTAMP WHERE id = ?3")
                    .bind(name).bind(sql).bind(id).execute(&self.metadata).await?.rows_affected();
                if changed == 0 { return Err("not found".into()); }
                Ok(id.into())
            }
            None => {
                let id = uuid::Uuid::now_v7().to_string();
                sqlx::query("INSERT INTO opensnow_worksheets (id, name, sql_text) VALUES (?, ?, ?)")
                    .bind(&id).bind(name).bind(sql).execute(&self.metadata).await?;
                Ok(id)
            }
        }
    }

    pub async fn save_dashboard(&self, id: Option<&str>, name: &str, worksheet_ids: &[String]) -> Result<Dashboard, BoxError> {
        let name = opensnow_control::analytics::dashboard_name(name)?;
        let ids = serde_json::to_string(worksheet_ids)?;
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM opensnow_worksheets WHERE id IN (SELECT value FROM json_each(?))",
        ).bind(&ids).fetch_one(&self.metadata).await?;
        if count != worksheet_ids.len() as i64 { return Err("not found".into()); }
        let (id, updated_at) = match id {
            Some(id) => sqlx::query_as::<_, (String, String)>(
                "UPDATE opensnow_dashboards SET name = ?, worksheet_ids = ?, updated_at = CURRENT_TIMESTAMP
                 WHERE id = ? RETURNING id, updated_at",
            ).bind(name).bind(ids).bind(id).fetch_optional(&self.metadata).await?.ok_or("not found")?,
            None => sqlx::query_as::<_, (String, String)>(
                "INSERT INTO opensnow_dashboards (id, name, worksheet_ids) VALUES (?, ?, ?) RETURNING id, updated_at",
            ).bind(uuid::Uuid::now_v7().to_string()).bind(name).bind(ids).fetch_one(&self.metadata).await?,
        };
        Ok(Dashboard { id, name: name.into(), worksheet_ids: worksheet_ids.to_vec(), updated_at, last_refresh: None })
    }

    pub(crate) async fn dashboards(&self, id: Option<&str>) -> Result<Vec<Dashboard>, BoxError> {
        let rows = sqlx::query_as::<_, (String, String, String, String)>(
            "SELECT id, name, worksheet_ids, updated_at FROM opensnow_dashboards
             WHERE (?1 IS NULL OR id = ?1) ORDER BY updated_at DESC, id",
        ).bind(id).fetch_all(&self.metadata).await?;
        rows.into_iter().map(|(id, name, ids, updated_at)| {
            Ok(Dashboard { id, name, worksheet_ids: serde_json::from_str(&ids)?, updated_at, last_refresh: None })
        }).collect()
    }

    /// Run a pinned worksheet and store its latest successful result in the local catalog.
    pub async fn refresh_dashboard_tile(&self, dashboard_id: &str, worksheet_id: &str) -> Result<Value, BoxError> {
        let sql = sqlx::query_scalar::<_, String>(
            "SELECT w.sql_text FROM opensnow_dashboards AS d
             JOIN json_each(d.worksheet_ids) AS tile
             JOIN opensnow_worksheets AS w ON w.id = tile.value WHERE d.id = ? AND w.id = ?",
        ).bind(dashboard_id).bind(worksheet_id).fetch_optional(&self.metadata).await?.ok_or("not found")?;
        let result = self.execute(&sql).await?;
        let mut data = crate::query_result_to_json(&result);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis() as u64;
        data["refreshed_at"] = json!(now);
        sqlx::query(
            "UPDATE opensnow_worksheets SET latest_tile_result = ? WHERE id = ? AND sql_text = ?",
        ).bind(data.to_string()).bind(worksheet_id).bind(&sql).execute(&self.metadata).await?;
        Ok(data)
    }

    pub(crate) async fn dashboard(&self, id: &str) -> Result<Value, BoxError> {
        let dashboard = self.dashboards(Some(id)).await?.pop().ok_or("not found")?;
        let rows = sqlx::query_as::<_, (String, String, String, Option<String>, String, String, Option<String>)>(
            "SELECT w.id, w.name, w.sql_text, w.chart_spec, w.created_at, w.updated_at, w.latest_tile_result
             FROM json_each(?) AS tile JOIN opensnow_worksheets AS w ON w.id = tile.value ORDER BY tile.key",
        ).bind(serde_json::to_string(&dashboard.worksheet_ids)?).fetch_all(&self.metadata).await?;
        let tiles = rows.into_iter().map(|(id, name, sql, chart, created_at, updated_at, latest)| {
            let chart_spec = chart.map(|s| serde_json::from_str::<Value>(&s)).transpose()?;
            let latest_result = latest.map(|s| serde_json::from_str::<Value>(&s)).transpose()?;
            Ok(json!({ "id": id, "name": name, "sql": sql, "chart_spec": chart_spec,
                "created_at": created_at, "updated_at": updated_at, "latest_result": latest_result }))
        }).collect::<Result<Vec<_>, BoxError>>()?;
        let mut data = json!(dashboard);
        data["tiles"] = json!(tiles);
        Ok(data)
    }
}
