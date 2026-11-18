"use strict";

const LITE_SCHEMA = "https://vega.github.io/schema/vega-lite/v5.json";
const chartTypes = new Map([
  ["bar", "Bar"], ["line", "Line"], ["area", "Area"], ["point", "Scatter"],
  ["arc", "Pie"], ["rect", "Heatmap"], ["table", "Table"],
]);
const renderedCharts = new WeakMap();
let activeChartSpec = null;
let worksheetSelection = 0;
let chartDraft = null;
let openDashboard = null;
let dashboardRequest = 0;
let dashboardEditor = null;
let pinWorksheetId = null;

function setMessage(id, message) {
  $(id).textContent = message;
  $(id).classList.toggle("hidden", !message);
}

function clearChart(container) {
  renderedCharts.get(container)?.embed?.finalize();
  renderedCharts.delete(container);
  container.replaceChildren();
}

async function drawChart(container, spec, result) {
  clearChart(container);
  if (!result) {
    container.textContent = "Run the worksheet to preview its result.";
    return;
  }
  if (!spec) {
    container.innerHTML = gridHtml(result);
    return;
  }
  const render = {};
  renderedCharts.set(container, render);
  const fieldNames = new Map(result.columns.map((column, i) => [column.name, "c" + i]));
  const rows = result.rows.map((row) => Object.fromEntries(result.columns.map((column, i) => {
    const value = row[i];
    return ["c" + i, fieldType(column) === "quantitative" && value !== null ? Number(value) : value];
  })));
  const display = structuredClone(spec);
  for (const field of Object.values(display.encoding)) {
    if (!field.field) continue;
    field.title ??= field.field;
    field.field = fieldNames.get(field.field) ?? "missing";
  }
  const target = document.createElement("div");
  target.className = "chart-renderer";
  container.append(target);
  try {
    const embed = await vegaEmbed(target, {
      ...display, data: { values: rows }, width: "container", height: 130,
    }, {
      mode: "vega-lite", renderer: "svg", actions: false, tooltip: false,
      config: { mark: { color: "#0284c7" }, view: { stroke: null } },
      loader: {
        load: async () => { throw new Error("A chart can only use its worksheet result."); },
        sanitize: async () => { throw new Error("A chart can only use its worksheet result."); },
      },
    });
    if (renderedCharts.get(container) === render) render.embed = embed;
    else embed.finalize();
  } catch (error) {
    if (renderedCharts.get(container) === render) {
      container.textContent = "Chart could not render. " + error.message;
    }
  }
}

function selectWorksheetChart(spec) {
  worksheetSelection++;
  activeChartSpec = spec;
  lastResult = null;
  hideRunPanels();
  $("panel-noresults").classList.remove("hidden");
  $("btn-chart-edit").textContent = spec ? "Edit chart" : "Add chart";
  clearChart($("worksheet-chart"));
  if ($("chart-editor").open) $("chart-editor").close();
}

function showWorksheetResult(chart) {
  const showChart = chart && Boolean(activeChartSpec);
  $("tab-chart").classList.toggle("hidden", !activeChartSpec);
  $("tab-chart").setAttribute("aria-selected", showChart);
  $("tab-results").setAttribute("aria-selected", !showChart);
  $("results-grid").classList.toggle("hidden", showChart);
  $("worksheet-chart").classList.toggle("hidden", !showChart);
  if (showChart) drawChart($("worksheet-chart"), activeChartSpec, lastResult);
  else clearChart($("worksheet-chart"));
}

$("tab-results").onclick = () => showWorksheetResult(false);
$("tab-chart").onclick = () => showWorksheetResult(true);
$("tab-history").onclick = () => switchView("history");

function button(label, action) {
  const element = document.createElement("button");
  element.type = "button";
  element.className = "btn small";
  element.textContent = label;
  element.onclick = action;
  return element;
}

function fieldType(column) {
  if (/^(?:U?Int|Float|Decimal)/.test(column?.type)) return "quantitative";
  if (/^(?:Date|Timestamp)/.test(column?.type)) return "temporal";
  return "nominal";
}

function setOptions(select, options, selected) {
  select.replaceChildren(...options.map(([label, value]) => new Option(label, value)));
  if ([...select.options].some((option) => option.value === selected)) select.value = selected;
}

function makeChartSpec() {
  if (!chartDraft.changed && chartDraft.original) return chartDraft.original;
  if (chartDraft.mark === "table") return null;
  const x = $("chart-x").value;
  const y = $("chart-y").value;
  if (!x || !y) return null;
  const group = $("chart-group").value;
  const field = (name) => ({ field: name, type: fieldType(chartDraft.columns.find((c) => c.name === name)) });
  const encoding = chartDraft.mark === "arc"
    ? { theta: { ...field(y), type: "quantitative" }, color: { field: group || x, type: "nominal" } }
    : { x: field(x), y: field(y) };
  if (chartDraft.mark === "rect") {
    encoding.x.type = encoding.y.type = "ordinal";
    encoding.color = group ? field(group) : { aggregate: "count", type: "quantitative" };
  } else if (group && chartDraft.mark === "arc") {
    encoding.detail = { field: x, type: "nominal" };
  } else if (group) {
    encoding.color = { field: group, type: "nominal" };
    if (chartDraft.mark === "bar") encoding.xOffset = encoding.color;
  }
  return { $schema: LITE_SCHEMA, mark: chartDraft.mark, encoding };
}

function updateChartEditor() {
  const table = chartDraft.mark === "table";
  for (const control of $("chart-types").children) {
    control.setAttribute("aria-pressed", control.dataset.mark === chartDraft.mark);
  }
  for (const id of ["chart-x", "chart-y", "chart-group"]) $(id).disabled = table;
  const spec = makeChartSpec();
  $("chart-save").disabled = !table && !spec;
  if (spec) {
    const { $schema, mark, encoding, ...rest } = spec;
    $("chart-json").textContent = JSON.stringify({ $schema, mark, encoding, ...rest }, null, 2);
  } else {
    $("chart-json").textContent = "null";
  }
  if (!chartDraft.original) drawChart($("chart-preview"), spec, chartDraft.result);
}

$("btn-chart-edit").onclick = () => {
  const original = activeChartSpec;
  const encoding = original?.encoding ?? {};
  const columns = [...(lastResult?.columns ?? [])];
  for (const def of Object.values(encoding)) {
    if (def.field && !columns.some((column) => column.name === def.field)) {
      columns.push({ name: def.field, type: def.type === "quantitative" ? "Float64" : def.type === "temporal" ? "Timestamp" : "Utf8" });
    }
  }
  const mark = original?.mark === "circle" ? "point" : original?.mark ?? "bar";
  chartDraft = { mark, columns, original, result: lastResult, changed: false };
  $("chart-title").textContent = original ? "Edit chart" : "Add a chart";
  $("chart-worksheet").textContent = $("ws-name").value + (lastResult ? " · " + lastResult.row_count + " rows" : " · run the worksheet to pick fields");
  $("chart-remove").classList.toggle("hidden", !original);
  $("chart-json-section").classList.toggle("hidden", !original);
  $("chart-preview").classList.toggle("hidden", Boolean(original));
  $("chart-preview-label").classList.toggle("hidden", Boolean(original));
  setMessage("chart-editor-error", "");
  $("chart-types").replaceChildren(...Array.from(chartTypes, ([type, label]) => {
    const control = button(label, () => {
      chartDraft.mark = type;
      chartDraft.changed = true;
      updateChartEditor();
    });
    control.dataset.mark = type;
    return control;
  }));
  const options = columns.map((column) => [column.name, column.name]);
  setOptions($("chart-x"), options, encoding.x?.field ?? encoding.detail?.field ?? encoding.color?.field);
  setOptions($("chart-y"), options, encoding.y?.field ?? encoding.theta?.field ?? columns[1]?.name);
  setOptions($("chart-group"), [["None", ""], ...options], mark === "arc" && !encoding.detail ? "" : encoding.color?.field);
  $("chart-editor").showModal();
  updateChartEditor();
};

for (const id of ["chart-x", "chart-y", "chart-group"]) {
  $(id).onchange = () => {
    chartDraft.changed = true;
    updateChartEditor();
  };
}

async function saveChart(spec) {
  const selection = worksheetSelection;
  $("chart-save").disabled = $("chart-remove").disabled = true;
  try {
    const saved = await callApi("POST", "/api/worksheets", {
      id: currentWorksheetId, name: $("ws-name").value, sql: $("ws-sql").value, chart_spec: spec,
    });
    if (selection !== worksheetSelection) return;
    currentWorksheetId = saved.id;
    activeChartSpec = saved.chart_spec;
    $("btn-chart-edit").textContent = spec ? "Edit chart" : "Add chart";
    $("chart-editor").close();
    if (lastResult) showWorksheetResult(Boolean(spec));
    await loadWorksheets();
  } catch (error) {
    setMessage("chart-editor-error", error.message);
  } finally {
    $("chart-save").disabled = $("chart-remove").disabled = false;
  }
}

$("chart-form").onsubmit = (event) => { event.preventDefault(); saveChart(makeChartSpec()); };
$("chart-remove").onclick = () => saveChart(null);
$("chart-cancel").onclick = () => $("chart-editor").close();
$("chart-editor").onclose = () => clearChart($("chart-preview"));

function resetAnalytics() {
  dashboardRequest++;
  openDashboard = null;
  dashboardEditor = null;
  chartDraft = null;
  pinWorksheetId = null;
  for (const id of ["dashboard-editor", "pin-editor", "chart-editor"]) $(id).close();
  selectWorksheetChart(null);
  currentWorksheetId = null;
  $("ws-name").value = "Untitled";
  $("ws-sql").value = "";
  $("results-grid").replaceChildren();
  $("results-cost").replaceChildren();
  clearDashboardTiles();
  $("dashboard-list").replaceChildren();
  $("dashboard-title").textContent = "";
  $("dashboard-view").classList.add("hidden");
}

function leaveDashboards() {
  dashboardRequest++;
  openDashboard = null;
  clearDashboardTiles();
}

function clearDashboardTiles() {
  for (const chart of $("dashboard-tiles").querySelectorAll(".chart")) clearChart(chart);
  $("dashboard-tiles").replaceChildren();
}

async function loadDashboards() {
  const request = ++dashboardRequest;
  openDashboard = null;
  clearDashboardTiles();
  $("view-app").classList.add("dashboard-list-open");
  $("dashboards-list-view").classList.remove("hidden");
  $("dashboard-view").classList.add("hidden");
  $("dashboard-new").classList.add("hidden");
  $("dashboard-list").replaceChildren();
  setMessage("dashboard-error", "");
  try {
    const { dashboards } = await callApi("GET", "/api/dashboards");
    if (request !== dashboardRequest) return;
    $("dashboard-count").textContent = dashboards.length;
    $("dashboard-list-subtitle").textContent = dashboards.length
      ? dashboards.length + (dashboards.length === 1 ? " dashboard" : " dashboards") + " · All times local"
      : "Pin saved worksheets and refresh their results together.";
    if (!dashboards.length) {
      const empty = document.createElement("div");
      empty.className = "empty";
      empty.innerHTML = "<h3>No dashboards yet</h3><p>Start from a worksheet you already run, then pin it here.</p>";
      const create = button("New dashboard", () => editDashboard());
      create.classList.add("primary");
      empty.append(create, " ", button("Open a worksheet", () => switchView("worksheets")));
      $("dashboard-list").append(empty);
      return;
    }
    $("dashboard-new").classList.remove("hidden");
    for (const dashboard of dashboards) {
      const row = document.createElement("article");
      row.className = "dashboard-row";
      row.innerHTML = '<span class="dashboard-icon" aria-hidden="true">▥</span><div><b></b><p class="sub"></p></div>';
      row.querySelector("b").textContent = dashboard.name;
      const count = dashboard.worksheet_ids.length;
      const refreshed = dashboard.last_refresh;
      row.querySelector(".sub").textContent = count + (count === 1 ? " tile" : " tiles") + " · " +
        (refreshed ? "Refreshed " + new Date(refreshed.refreshed_at).toLocaleString() : "Never refreshed");
      const cost = document.createElement("span");
      cost.className = "dashboard-refresh-cost";
      cost.textContent = refreshed?.cost_dollars == null
        ? (refreshed ? "No cost recorded" : "No runs yet")
        : "Last refresh " + dollars(refreshed.cost_dollars);
      const open = button("Open", () => showDashboard(dashboard.id, true));
      open.classList.add("dashboard-actions");
      open.setAttribute("aria-label", "Open " + dashboard.name);
      row.append(open, cost);
      $("dashboard-list").append(row);
    }
  } catch (error) {
    if (request === dashboardRequest) setMessage("dashboard-error", error.message);
  }
}

async function showDashboard(id, refresh) {
  const request = ++dashboardRequest;
  setMessage("dashboard-error", "");
  try {
    const dashboard = await callApi("GET", "/api/dashboards/" + encodeURIComponent(id));
    if (request !== dashboardRequest) return;
    openDashboard = dashboard;
    $("view-app").classList.remove("dashboard-list-open");
    $("dashboard-title").textContent = dashboard.name;
    $("dashboards-list-view").classList.add("hidden");
    $("dashboard-view").classList.remove("hidden");
    clearDashboardTiles();
    for (const tile of dashboard.tiles) {
      const card = document.createElement("article");
      card.className = "tile";
      card.innerHTML = '<div class="tile-head"><b></b></div><div class="tile-body"><div class="tile-state"></div><div class="chart"></div></div><div class="tile-foot"></div>';
      card.querySelector("b").textContent = tile.name;
      tile.card = card;
      tile.refresh = button("Refresh", () => refreshTile(dashboard, tile));
      tile.refresh.classList.add("dashboard-actions");
      card.querySelector(".tile-head").append(tile.refresh);
      $("dashboard-tiles").append(card);
      paintTile(tile);
    }
    updateDashboardStatus();
    if (refresh) refreshDashboard();
  } catch (error) {
    if (request === dashboardRequest) setMessage("dashboard-error", error.message);
  }
}

function dollars(value) {
  const [digits, exponent] = String(value).split("e-");
  if (!exponent) return "$" + digits;
  return "$0." + "0".repeat(Number(exponent) - 1) + digits.replace(".", "");
}

function paintTile(tile) {
  const state = tile.card.querySelector(".tile-state");
  const chart = tile.card.querySelector(".chart");
  const footer = tile.card.querySelector(".tile-foot");
  state.replaceChildren();
  state.className = "tile-state";
  footer.replaceChildren();
  const result = tile.latest_result;
  tile.refresh.disabled = Boolean(tile.running);
  tile.refresh.textContent = tile.error ? "Retry" : "Refresh";
  tile.card.setAttribute("aria-busy", Boolean(tile.running));
  if (tile.running) {
    clearChart(chart);
    state.innerHTML = '<div class="tile-spinner" aria-hidden="true"></div><p class="tile-running">Running query…</p>';
    updateTileElapsed(tile);
    footer.textContent = "Cost shown when done";
    return;
  }
  if (tile.error) {
    const cap = tile.error.data?.cost_cap;
    state.classList.add("panel", cap ? "amber" : "red");
    const heading = document.createElement("b");
    heading.textContent = cap ? "Cost cap reached. " : "Refresh failed. ";
    state.append(heading, cap ? "An admin can raise the tenant cost cap to resume refreshes." : tile.error.message);
    const record = tile.error.data?.cost_record;
    if (record) footer.textContent = "Refresh cost " + dollars(record.cost_dollars);
    else if (Object.hasOwn(tile.error.data ?? {}, "cost_record")) footer.textContent = "No cost recorded for this refresh";
  }
  if (result) {
    const time = result.refreshed_at ? " · " + new Date(result.refreshed_at).toLocaleTimeString() : "";
    const label = document.createElement("span");
    label.textContent = (tile.error ? "Showing last good result" : "Refreshed") + time;
    const cost = document.createElement("span");
    cost.className = "mono";
    cost.textContent = (tile.error ? "Last good result cost " : "Cost ") + dollars(result.outcome.cost_dollars);
    footer.append(label, cost);
    drawChart(chart, tile.chart_spec, result);
  } else if (!tile.error) {
    footer.textContent = "No refresh yet";
  }
}

function updateDashboardStatus() {
  if (!openDashboard) return;
  const { tiles } = openDashboard;
  const running = tiles.filter((tile) => tile.running).length;
  const failures = tiles.filter((tile) => tile.error).length;
  const cap = tiles.some((tile) => tile.error?.data?.cost_cap);
  $("dashboard-refresh").disabled = Boolean(running) || !tiles.length;
  $("dashboard-status").textContent = running ? "Refreshing " + running + " of " + tiles.length + " tiles"
    : cap ? "Refresh blocked by the tenant cost cap"
    : failures ? failures + (failures === 1 ? " tile failed on refresh" : " tiles failed on refresh")
    : tiles.length ? tiles.length + (tiles.length === 1 ? " tile" : " tiles") : "Add saved worksheets to start.";
}

function updateTileElapsed(tile) {
  const elapsed = ((performance.now() - tile.started) / 1000).toFixed(1);
  tile.card.querySelector(".tile-running").textContent = "Running query… elapsed " + elapsed + "s";
}

async function refreshTile(dashboard, tile) {
  if (tile.running) return;
  tile.started = performance.now();
  tile.running = true;
  tile.error = null;
  paintTile(tile);
  updateDashboardStatus();
  const timer = setInterval(() => updateTileElapsed(tile), 100);
  try {
    const result = await callApi("POST", "/api/dashboards/refresh-tile", { dashboard_id: dashboard.id, worksheet_id: tile.id });
    tile.chart_spec = result.chart_spec;
    tile.latest_result = result;
  } catch (error) {
    tile.error = error;
  } finally {
    clearInterval(timer);
    tile.running = false;
    if (openDashboard === dashboard) {
      paintTile(tile);
      updateDashboardStatus();
    }
  }
}

async function refreshDashboard() {
  const dashboard = openDashboard;
  if (!dashboard?.tiles.length || dashboard.tiles.some((tile) => tile.running)) return;
  for (const tile of dashboard.tiles) {
    tile.started = performance.now();
    tile.running = true;
    tile.error = null;
    paintTile(tile);
  }
  updateDashboardStatus();
  const timer = setInterval(() => dashboard.tiles.forEach(updateTileElapsed), 100);
  try {
    const refresh = await callApi("POST", "/api/dashboards/refresh", { dashboard_id: dashboard.id });
    const byId = new Map(refresh.tiles.map((tile) => [tile.worksheet_id, tile]));
    dashboard.last_refresh = refresh.last_refresh;
    for (const tile of dashboard.tiles) {
      const run = byId.get(tile.id);
      if (run?.status === 200) {
        tile.chart_spec = run.data.chart_spec;
        tile.latest_result = run.data;
      } else {
        tile.error = { message: run?.data.error ?? "Open the dashboard again to load the current tiles.", data: run?.data };
      }
    }
  } catch (error) {
    for (const tile of dashboard.tiles) tile.error = error;
  } finally {
    clearInterval(timer);
    for (const tile of dashboard.tiles) {
      tile.running = false;
      if (openDashboard === dashboard) paintTile(tile);
    }
    if (openDashboard === dashboard) updateDashboardStatus();
  }
}

$("dashboard-nav-catalog").onclick = () => switchView("navigate");
$("dashboard-nav-worksheets").onclick = () => switchView("worksheets");
$("dashboard-nav-dashboards").onclick = loadDashboards;
$("dashboard-nav-history").onclick = () => switchView("history");
$("dashboard-back").onclick = loadDashboards;
$("dashboard-refresh").onclick = refreshDashboard;
$("dashboard-new").onclick = () => editDashboard();
$("dashboard-edit").onclick = () => editDashboard(openDashboard);

async function editDashboard(dashboard, worksheetId) {
  setMessage("dashboard-form-error", "");
  setMessage("dashboard-error", "");
  try {
    const { worksheets } = await callApi("GET", "/api/worksheets");
    const ids = dashboard?.worksheet_ids ?? (worksheetId ? [worksheetId] : []);
    const selected = new Set(ids);
    const byId = new Map(worksheets.map((worksheet) => [worksheet.id, worksheet]));
    const ordered = [...ids.map((id) => byId.get(id)).filter(Boolean), ...worksheets.filter((worksheet) => !selected.has(worksheet.id))];
    dashboardEditor = { id: dashboard?.id, ordered, selected };
    $("dashboard-editor-title").textContent = dashboard ? "Edit dashboard" : "New dashboard";
    $("dashboard-name").value = dashboard?.name ?? "";
    $("dashboard-delete").classList.toggle("hidden", !dashboard);
    showDashboardOptions();
    $("dashboard-editor").showModal();
  } catch (error) {
    const target = $("section-dashboards").classList.contains("hidden") ? "pin-error" : "dashboard-error";
    setMessage(target, error.message);
  }
}

function showDashboardOptions() {
  const { ordered, selected } = dashboardEditor;
  const items = ordered.map((worksheet, index) => {
    const row = document.createElement("div");
    row.className = "dashboard-pick-row";
    const input = document.createElement("input");
    input.type = "checkbox";
    input.checked = selected.has(worksheet.id);
    input.id = "pick-" + worksheet.id;
    input.onchange = () => input.checked ? selected.add(worksheet.id) : selected.delete(worksheet.id);
    const label = document.createElement("label");
    label.htmlFor = input.id;
    label.textContent = worksheet.name;
    const move = (direction, title) => {
      const control = button(title, () => {
        const next = index + direction;
        [ordered[index], ordered[next]] = [ordered[next], ordered[index]];
        showDashboardOptions();
        $(input.id).focus();
      });
      control.disabled = index + direction < 0 || index + direction >= ordered.length;
      control.setAttribute("aria-label", title + " " + worksheet.name);
      return control;
    };
    row.append(input, label, move(-1, "Move up"), move(1, "Move down"));
    return row;
  });
  $("dashboard-worksheets").replaceChildren(...items);
}

$("dashboard-form").onsubmit = async (event) => {
  event.preventDefault();
  const { id, ordered, selected } = dashboardEditor;
  const worksheet_ids = ordered.filter((worksheet) => selected.has(worksheet.id)).map((worksheet) => worksheet.id);
  $("dashboard-save").disabled = true;
  try {
    const saved = await callApi("POST", "/api/dashboards", { id, worksheet_ids, name: $("dashboard-name").value });
    $("dashboard-editor").close();
    if (id) showDashboard(saved.id, false);
    else if (!$("section-dashboards").classList.contains("hidden")) loadDashboards();
  } catch (error) {
    setMessage("dashboard-form-error", error.message);
  } finally {
    $("dashboard-save").disabled = false;
  }
};

$("dashboard-cancel").onclick = () => $("dashboard-editor").close();
$("dashboard-delete").onclick = async () => {
  if (!confirm("Delete this dashboard?")) return;
  $("dashboard-delete").disabled = true;
  try {
    await callApi("POST", "/api/dashboards/delete", { id: dashboardEditor.id });
    $("dashboard-editor").close();
    loadDashboards();
  } catch (error) {
    setMessage("dashboard-form-error", error.message);
  } finally {
    $("dashboard-delete").disabled = false;
  }
};

$("btn-pin").onclick = async () => {
  if (!currentWorksheetId) {
    showError(new Error("Save the worksheet before pinning it."), -1);
    return;
  }
  try {
    const { dashboards } = await callApi("GET", "/api/dashboards");
    pinWorksheetId = currentWorksheetId;
    setMessage("pin-error", "");
    setOptions($("pin-dashboard"), [...dashboards.map((dashboard) => [dashboard.name, dashboard.id]), ["New dashboard…", ""]]);
    $("pin-editor").showModal();
  } catch (error) {
    if (error.status !== 401) showError(error, -1);
  }
};

$("pin-cancel").onclick = () => $("pin-editor").close();
$("pin-form").onsubmit = async (event) => {
  event.preventDefault();
  const id = $("pin-dashboard").value;
  if (!id) {
    $("pin-editor").close();
    await editDashboard(null, pinWorksheetId);
    return;
  }
  $("pin-submit").disabled = true;
  try {
    const dashboard = await callApi("GET", "/api/dashboards/" + encodeURIComponent(id));
    const worksheet_ids = [...new Set([...dashboard.worksheet_ids, pinWorksheetId])];
    await callApi("POST", "/api/dashboards", { id, name: dashboard.name, worksheet_ids });
    $("pin-editor").close();
  } catch (error) {
    setMessage("pin-error", error.message);
  } finally {
    $("pin-submit").disabled = false;
  }
};


/* Resume a session stored in sessionStorage. */
(async () => {
  if (!token) return;
  try {
    sessionInfo = await callApi("GET", "/api/session");
    showApp();
  } catch (error) {
    if (error.status !== 401) {
      token = "";
      sessionStorage.removeItem("osn_token");
    }
  }
})();
