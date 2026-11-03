# jobs/ — jobs and schedules

## Purpose

The operational record: what ran, what is running, what failed and why.

## Why this is a screen and not a toast

A bulk action over fifty sandboxes can half-succeed. A toast that says "done"
loses the half that failed. A job keeps the per-target result, so the retry is
informed rather than blind.

## Files

| File | What it does |
|---|---|
| `JobsPage.jsx` | Tasks page: filter chips, progress table, schedules drawer; `?id=` opens a task. |
| `JobDrawer.jsx` | One task: facts, per-target results, raw log, retry of failed targets (bulk kinds only). |
| `jobLabels.js` | `labelFor`, `toneFor`, `summaryLine` (used by the dock), plus pure helpers for target results and duration. |
| `jobLabels.test.js` | Tests for the labels, summary line and target parsing. |
