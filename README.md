# ProjectCraft

Project planning with dependencies, CPM and Gantt

**ProjectCraft is an independent, community open-source project.** It is not an
official product of, endorsed by, or affiliated with any third-party vendor it
may interoperate with.

Built on [CraftKit](https://github.com/emilio-n8n/craftkit), a small native Rust
desktop foundation.

## Features

- Native Rust desktop UI (egui/eframe), no web runtime.
- Editable, sortable task table: name, start day, duration, dependencies (`1,2`)
  and progress. Add, duplicate, reorder and delete tasks; deleting one repairs
  the dependency graph.
- Critical Path Method in whole days: earliest/latest start and finish, total
  float and critical-path detection with clear cycle diagnostics.
- Interactive Gantt drawn with egui's `Painter`: dependency arrows, clickable
  row labels, precise bar hit-testing, bar dragging to reschedule from the
  visible CPM start, day/week/month/fit zoom, scrolling, progress overlay,
  slack markers and colour-coded critical tasks.
- Optional calendar: a project start date and weekend skipping display real
  dates on the ruler without changing integer-day scheduling.
- Task milestones (zero-duration diamonds), notes, resources and estimated
  costs; the total cost is shown in the status line.
- CSV import and export with strict quoting, column validation, exact authored
  value round-trips, dependency checks and descriptive row errors.
- Paginated vector PDF export of the Gantt for printing, with no extra
  dependency and atomic file replacement.
- Versioned JSON project files with atomic saves and schema validation.
- Bounded undo/redo through the shared document history; bar and numeric drags
  are each one step, with unsaved-change confirmation before closing.
- Recent files and persisted preferences.
- Offline-first: no account, no telemetry, no network required.

## Run

```bash
cargo run --release -- --sample
cargo run --release -- --open project.json
```

Without arguments the application opens an empty document.

- `--sample` opens the built-in demonstration document.
- `--open <file>` opens a project.
- `--version` prints the version.
- `--help` prints usage.

## Build and test

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
```

Linux windowing development packages are installed by the CI workflow.

## Project layout

```text
src/main.rs       CLI and window startup
src/app.rs        UI, interactive Gantt and project lifecycle
src/document.rs   task/document model and field parsing
src/schedule.rs   CPM forward/backward passes and validation
src/calendar.rs   optional real dates and weekend skipping
src/gantt.rs      zoom, hit-testing and arrow geometry
src/csv.rs        CSV import/export with quoting
src/pdf.rs        dependency-free vector Gantt export
```

The application depends on pinned CraftKit crates from Git. Keep the tag in
`Cargo.toml` and `Cargo.lock` consistent when upgrading the foundation.

## File format

Projects are JSON envelopes:

```json
{
  "format": "projectcraft",
  "schema_version": 1,
  "app_version": "0.1.0",
  "document": {
    "title": "Sample project",
    "tasks": [
      {
        "id": 1,
        "name": "Requirements",
        "start_day": 0,
        "duration_days": 2,
        "dependencies": [],
        "progress": 1.0,
        "milestone": false,
        "notes": "",
        "resource": "",
        "cost": 0.0
      }
    ],
    "next_id": 2,
    "calendar": { "start_date": null, "skip_weekends": false }
  }
}
```

Days are signed integers; `start_day` is a release constraint and durations are
whole days. The Gantt and PDF start their ruler at the first scheduled day, so
plans before day zero remain visible. The optional `calendar` maps day offsets
to real dates for display.
Fields added in v0.2 have serde defaults, so schema 1 files written by v0.1.0
still open without a migration. A newer `schema_version` is rejected instead of
being silently misread.

## Licence

Licensed under either of:

- Apache License, Version 2.0 (`LICENSE-APACHE`)
- MIT license (`LICENSE-MIT`)

at your option.

## Third-party notices

See [THIRD_PARTY.md](THIRD_PARTY.md).
