# Changelog — ProjectCraft

All notable changes to ProjectCraft are documented here.

## [0.2.0] - 2026-11-22

### Added

- Interactive Gantt: dependency arrows, task selection by row, and bar dragging
  that reschedules a task with a single undo step.
- Zoom modes (Day, Week, Month, Fit) with horizontal/vertical scrolling.
- Optional real-date calendar over integer days, with weekend skipping.
- Task milestones, notes, resources and estimated costs, with a details panel.
- CSV import alongside export, with quoted fields and clear row errors.
- Vector PDF export of the Gantt chart (no extra dependencies).
- Sortable task table and explicit empty states.
- Tests for drag gestures, arrow geometry, calendars, CSV import, PDF structure,
  JSON round-trip and undo.

### Changed

- Additive fields use serde defaults: v0.1.0 schema 1 projects still open.

## [0.1.0] - 2026-11-22

### Added

- Initial CraftKit-based application shell.
- Task document model with a five-task sample project.
- Critical Path Method scheduler in whole days: earliest/latest dates, total
  float, critical path and explicit cycle diagnostics.
- Editable task table with dependencies, progress and add/delete actions.
- Gantt canvas with day ruler, critical-path colouring and slack markers.
- CSV export of authored fields and computed CPM dates.
- Versioned project files with atomic save and bounded undo/redo.
