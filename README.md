# ProjectCraft

Project planning with dependencies, CPM and Gantt

**ProjectCraft is an independent, community open-source project.** It is not an
official product of, endorsed by, or affiliated with any third-party vendor it
may interoperate with.

Built on [CraftKit](https://github.com/emilio-n8n/craftkit), a small native Rust
desktop foundation.

## Features

- Native Rust desktop UI (egui/eframe), no web runtime.
- Editable task table: name, start day, duration, dependencies (`1,2`) and
  progress. Add and delete tasks; deleting one repairs the dependency graph.
- Critical Path Method in whole days: earliest/latest start and finish, total
  float and critical-path detection with clear cycle diagnostics.
- Gantt canvas drawn with egui's `Painter`: day ruler, per-task bars, progress
  overlay, slack markers and colour-coded critical tasks.
- CSV export of authored fields plus computed CPM dates, with proper quoting.
- Versioned JSON project files with atomic saves and schema validation.
- Bounded undo/redo through the shared document history.
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
cargo test --workspace
```

Linux windowing development packages are installed by the CI workflow.

## Project layout

```text
src/main.rs       CLI and window startup
src/app.rs        UI, menus and project lifecycle
src/document.rs   task/document model and field parsing
src/schedule.rs   CPM forward/backward passes and validation
src/csv.rs        CSV export with quoting
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
        "progress": 1.0
      }
    ],
    "next_id": 2
  }
}
```

Days are signed integers (no calendar dates), `start_day` is a release
constraint and durations are whole days. The application owns the `document`
schema and its migrations. A newer `schema_version` is rejected instead of
being silently misread.

## Licence

Licensed under either of:

- Apache License, Version 2.0 (`LICENSE-APACHE`)
- MIT license (`LICENSE-MIT`)

at your option.

## Third-party notices

See [THIRD_PARTY.md](THIRD_PARTY.md).
