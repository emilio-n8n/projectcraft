# Decisions — ProjectCraft

Short architecture decision records. Newest first.

## ADR-004 — Optional calendar without changing CPM

Date: 2026-11-22

Decision: keep the critical-path calculation in integer project days and add a
`calendar` object that only maps day offsets to civil dates, optionally skipping
weekends. Date arithmetic lives in `src/calendar.rs` and uses no external date
crate.

Alternatives: convert the whole scheduler to real dates; add `chrono`.

Reasons: the existing CPM tests and JSON remain valid; day-only maths stays
deterministic and cheap; an optional start date keeps new projects simple.

Consequences: weekend skipping affects ruler labels and exports, not task
durations or float. New document/task fields use `#[serde(default)]`, so v0.1.0
schema 1 files still load without a migration and the schema version stays 1.

## ADR-003 — Dependency-free vector PDF export

Date: 2026-11-22

Decision: write the Gantt PDF by hand in `src/pdf.rs` (single page, Helvetica,
vector bars, arrows and text) instead of adding a PDF crate.

Alternatives: add `printpdf`/`lopdf`; render a screenshot to an image.

Reasons: no extra dependency, deterministic output, testable structure and
small files. A vector page prints cleanly at any size.

Consequences: only ASCII text is emitted (non-ASCII characters become `?`);
there is one landscape A4 page and no pagination for very large plans.

## ADR-002 — Initial document model

Date: 2026-11-22

Decision: store a project as a title, a list of tasks and a `next_id` counter.
Task identifiers are stable `u64`; start and duration are signed whole days;
dependencies are task ids. Later fields (milestone, notes, resource, cost,
calendar) are additive with serde defaults.

Alternatives: date-based scheduling, string identifiers, implicit ids derived
from vector position.

Reasons: deterministic integer maths, easy serialisation, stable references
from dependencies when tasks are reordered.

Consequences: deleting a task repairs dependency lists; a schema bump is only
needed for breaking changes, not for the additive v0.2 fields.

## ADR-001 — Built on CraftKit

Date: 2026-11-22

Decision: use the pinned CraftKit crates for the native shell, project
lifecycle, history and background jobs.

Alternatives: build the desktop foundation from scratch; use another UI
framework.

Reasons: consistent behaviour across the portfolio, fewer duplicated bugs,
shared release and testing conventions.

Consequences: CraftKit upgrades are explicit tag changes; domain code stays in
this repository.
