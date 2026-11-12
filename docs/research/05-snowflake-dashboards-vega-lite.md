# opensnow: Snowflake Dashboards and the Vega-Lite Chart Protocol

*Field note, 2026-11-12. Trigger: Snowflake's Expedition launch (November 2026) announced a dedicated dashboard capability. Recorded for the Phase 2 analytics slice. Nothing here changes Phase 1, the #11 rulings, or the build queue.*

## What Snowflake announced

- **Snowflake Dashboards**: users ask conversational questions, generate charts, and pin them into dashboards. Positioned as a direct challenger to Tableau and Power BI. No per-seat licensing; it runs on warehouse consumption. Migration tools auto-convert legacy BI artifacts. Source: Snowflake Builders Blog, "Inside Snowflake 'Expedition' Launch" (index) — https://medium.com/snowflake/inside-snowflake-expedition-launch-summary-of-new-features-announced-5b049379fd2b
- **Vega-Lite is the chart format across Snowflake's AI surface**, not just the dashboard renderer. CoWork (formerly Snowflake Intelligence) generates Vega-Lite charts and supports most Vega-Lite types, including layered, faceted, dual-axis, heatmaps, and box plots. Source: Snowflake documentation, Overview of Snowflake CoWork (index) — https://docs.snowflake.com/en/user-guide/snowflake-cortex/snowflake-cowork
- **Cortex Agents return charts as Vega-Lite v5 JSON specs** in the response, a stringified JSON document the client renders. As one analysis puts it, Vega-Lite acts as a protocol between the agent and the UI, not just a charting preference. Source: (index) — https://www.jodybrewster.dev/research/snowflake-cortex-learning-customization-charts

## What it means for opensnow

1. **It validates D-03 and non-negotiable 4.** Dashboards are becoming part of the warehouse surface, not a separate BI purchase. Snowflake spending launch capital on a Tableau and Power BI challenger says the experience layer (catalog, worksheets, analytics) is where competition moves next, which is the bet opensnow already made.
2. **The counter-position still holds.** Snowflake dashboards run on the credit meter, so every pinned chart and every refresh bills like any other warehouse work. opensnow serves the same surface without that tax, at the Gate 1 measured cost base.
3. **Vega-Lite is the strong default for the analytics slice.** A chart as a JSON spec is something an agent can generate, diff, version, and store next to the worksheet that produced it. That is the same property that makes SQL agent-friendly, applied to visualization, and it matches non-negotiable 1 (AI-first, agent-generatable). Adopting the same open format would also make opensnow charts interchangeable with the specs Snowflake's own agents emit, which lowers migration cost for users coming over.

## Open checks for design time

- **License**: Vega-Lite is BSD-3-Clause. Permissive, but it needs an explicit pass through the D-04 license check before any dependency is taken. Do not assume it is cleared.
- **Scope**: which Vega-Lite chart types the analytics slice supports in v1, and how dashboard refresh is scheduled and metered, are design questions for the slice, not settled here.

## What does not change

- Q6 stands: no charts in #11. Worksheets ship as built.
- Sequencing stands: navigate and worksheets first (done in Phase 1), analytics next, built-in dbt after, hybrid tables last.
- No issue is filed from this note. It is evidence for the Phase 2 analytics design, to be read when that design starts.
