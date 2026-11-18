# Analytics

Run a worksheet and choose **Add chart**. Pick bar, line, area, scatter,
pie, heatmap, or table, then choose fields from the result. **Save
chart** saves the chart and the SQL together. Results and Chart show
the same run and share its cost. The chart editor shows the saved
Vega-Lite JSON.

**Pin to dashboard** adds a saved worksheet to a dashboard. Use **Edit
dashboard** to add, remove, or reorder tiles. Each tile reads its
worksheet's current chart. A worksheet without a chart shows a table.

Opening a dashboard runs its tiles. **Refresh all** reruns each tile,
and **Refresh** reruns one. Every run goes through the same authenticated
execute path as a worksheet. The tile shows the exact cost after the
run. If a refresh fails, the tile keeps its last successful result and
shows the failed run's cost when the engine recorded one. A cap
rejection before execution has no cost record. The dashboard list
shows when the last full refresh finished and its recorded cost.

Worksheets and dashboards belong to one user in one tenant. Postgres
stores both, including the last successful result for a refreshed
worksheet. A role or session error is handled by the existing token
checks. The same MCP tools work over the local SQLite catalog.
Embedded callers can create local worksheets and dashboards with
`LocalDb::save_worksheet` and `LocalDb::save_dashboard`.

## API

- `GET /api/chart-spec/{worksheet_id}` reads a spec. `POST /api/chart-spec`
  takes `worksheet_id` and `chart_spec`. Send null to remove a chart.
  Worksheet saves also accept `chart_spec`. Omit the field to keep the
  current chart.
- `GET /api/dashboards` lists dashboards. `GET /api/dashboards/{id}`
  returns a dashboard and its ordered tiles. `POST /api/dashboards` takes
  `name`, ordered `worksheet_ids`, and an optional `id` for updates.
  `POST /api/dashboards/delete` takes `id`.
- `POST /api/dashboards/refresh` takes `dashboard_id` and returns each
  tile's status and result. `POST /api/dashboards/refresh-tile` also
  takes `worksheet_id` and runs just that tile.
- MCP tools are `get_chart_spec`, `set_chart_spec`, `list_dashboards`, and
  `get_dashboard`. Chart tools take `worksheet_id` and dashboard
  detail takes `dashboard_id`. Hosted MCP chart writes require a
  write-scope token. The HTTP worksheet editor uses the same storage
  permissions as worksheet saves.

The stored format is a Vega-Lite v5 unit chart over worksheet fields.
The renderer takes its rows from the query. The supported mark and
encoding fields are validated before saves.

## Renderer license

D-04 passed before the build. Vega 5.33.0, Vega-Lite 5.23.0, and
Vega-Embed 6.29.0 are BSD-3-Clause. The repository vendors their browser
bundles and the server serves them locally. The browser loads every asset from the
OpenSnow server. The [renderer inventory](../vendor/vega/dependency-licenses.tsv)
lists the bundled packages and their retained license texts.
