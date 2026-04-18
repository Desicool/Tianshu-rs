---
name: dashboard-e2e
description: Runs Playwright-based E2E tests for the tianshu-dashboard web UI.
  Use after implementing or modifying tianshu-dashboard. Verifies the browser UI
  renders correctly and that displayed statistics match the actual data in the store.
---

# dashboard-e2e Agent

## Capabilities

Tools: Bash, Read, Glob, Grep, playwright MCP tools (browser_navigate, browser_snapshot,
browser_click, browser_take_screenshot, browser_evaluate, browser_wait_for)

## Never modify source code

This agent only tests and reports. Failures are routed back to feature-implementer.

## Workflow

1. Build: `cargo build -p approval_workflow 2>&1`
2. Start server: `cargo run -p approval_workflow -- --dashboard 8080 &` (save PID)
3. Wait for ready: poll `GET http://127.0.0.1:8080/api/config` until HTTP 200 (max 30s)
4. Run B1–B10 test scenarios (see below)
5. After each test: `browser_take_screenshot` — save path in report
6. Stop server: `kill $PID`
7. Report: table of Scenario / PASS|FAIL / Notes / Screenshot

## Test Scenarios (B1–B10)

### B1 — Page loads
- Navigate to `http://127.0.0.1:8080`
- Assert: page title contains "Tianshu Dashboard"
- Assert: 4 KPI cards visible (Running, Waiting, Finished, Rate)
- Assert: cases table with header row present

### B2 — KPI card values
- Pre-condition: server started with 3 Running + 7 Waiting cases in store
- Assert: KPI card labeled "Running" shows value "3"
- Assert: KPI card labeled "Waiting" shows value "7"

### B3 — Stats bar renders
- Pre-condition: completion rate ≈ 75%
- Assert: stats bar for "Completion" has inline width style ≈ 75% (±5%)

### B4 — State filter
- Click the "Waiting" option in the filter dropdown
- Assert: all visible table rows have state badge containing "Waiting"
- Assert: no "Running" badge rows visible

### B5 — Row expansion
- Click on any table row
- Assert: expanded details section appears containing "Session:" label
- Assert: resource_data JSON block is rendered in a `<pre>` element

### B6 — case_key copy
- Click on the case_key chip in any row
- Assert: element has `data-copied="true"` attribute set (the JS sets this as feedback)

### B7 — Pagination
- Pre-condition: 50 active cases in store
- Assert: pagination shows "Page 1 of 3" and "(showing 20 of 50)"
- Click "Next →" button
- Assert: pagination shows "Page 2 of 3"
- Click "← Prev" button
- Assert: back to "Page 1 of 3"

### B8 — Period tab switch
- Click the "3d" period button
- Wait for fetch to complete (browser_wait_for network idle or 500ms)
- Assert: stats section shows `window_days: 3` in the fetched data (via browser_evaluate)

### B9 — Auto-refresh
- Read current Running count from KPI card
- Inject a new Running case via direct API call (POST or internal mechanism)
- Wait 6 seconds (browser_wait_for timeout)
- Assert: Running count has incremented by 1 without page reload

### B10 — Error banner
- Note current state
- Stop the server process
- Wait for next refresh cycle (≤ refresh_secs + 1s)
- Assert: error banner element is visible with text containing "unavailable"

## Exit criteria

All B1–B10 pass → final message: "E2E: ALL PASS"
Any failure → final message lists: scenario ID, expected, actual, screenshot path, suggested fix
