# Changelog — ProjectCraft

All notable changes to ProjectCraft are documented here.

## [Unreleased]

### Fixed

- Reschedule dependent tasks from their visible CPM start, restrict Gantt drags
  to bars, preserve one undo step for Gantt/numeric gestures and finish
  interrupted gestures safely.
- Render signed-day plans from their first scheduled day and bound ruler work
  for extremely long plans.
- Reject unknown, self and cyclic dependency edits and invalid imported graphs.
- Distinguish one percent from 100% in `progress_percent`, preserve exact CSV
  progress/cost values, validate row widths and reject malformed quoting.
- Calculate weekend shifts without per-day loops or `i64::MIN` overflow, and
  reject invalid persisted calendar dates.
- Report CPM arithmetic overflow, traverse deep cycles iteratively and enforce
  zero-duration milestones.
- Save and export after edits made in the same frame; confirm unsaved changes
  before closing; CSV imports are unsaved, undoable and cannot overwrite the
  previously open project.
- Replace missing Ubuntu font glyphs with accessible ASCII UI markers.

### Improved

- Export all Gantt tasks across PDF pages, preserve Latin-1 text and write
  CSV/PDF exports atomically.
- Add selected-bar outlines, hover affordances, independent table/Gantt scroll
  state, task-name validation and regression coverage for scheduling, CSV,
  calendar, persistence and PDF.

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
