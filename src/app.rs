//! ProjectCraft UI: task table, interactive Gantt and project lifecycle.

use std::collections::BTreeMap;
use std::path::PathBuf;

use craftkit_io::DocumentHost;
use craftkit_ui::cli::CliOptions;
use craftkit_ui::eframe::{self, egui};
use craftkit_ui::egui::{
    Align, Align2, CornerRadius, DragValue, FontFamily, FontId, Layout, Rect, RichText, ScrollArea,
    Sense, Shape, Stroke, TextEdit, Vec2, pos2, vec2,
};
use craftkit_ui::{AppIdentity, AppShell, Tone, dialogs, theme, widgets};

use crate::calendar::Date;
use crate::csv::{export_csv as write_csv, import_csv};
use crate::document::{Document, format_dependencies, parse_dependencies};
use crate::gantt::{
    HEADER_HEIGHT, LABEL_WIDTH, ROW_HEIGHT, Zoom, arrow_points, bar_rect, day_x, drag_delta_days,
    row_at,
};
use crate::pdf::export_pdf;
use crate::schedule::{Schedule, schedule};

const FORMAT: &str = "projectcraft";
const SCHEMA_VERSION: u32 = 1;
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Fixed cell widths of the task table.
const ID_WIDTH: f32 = 40.0;
const NAME_WIDTH: f32 = 190.0;
const START_WIDTH: f32 = 56.0;
const DURATION_WIDTH: f32 = 56.0;
const DEPENDENCIES_WIDTH: f32 = 86.0;
const PROGRESS_WIDTH: f32 = 106.0;
const DELETE_WIDTH: f32 = 26.0;
const CELL_HEIGHT: f32 = 22.0;
const TABLE_WIDTH: f32 = ID_WIDTH
    + NAME_WIDTH
    + START_WIDTH
    + DURATION_WIDTH
    + DEPENDENCIES_WIDTH
    + PROGRESS_WIDTH
    + DELETE_WIDTH
    + 6.0 * 8.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    New,
    Open,
    Save,
    SaveAs,
    ImportCsv,
    ExportCsv,
    ExportPdf,
    Undo,
    Redo,
    About,
    Settings,
}

#[derive(Debug, Clone)]
enum PendingAction {
    New,
    Open(PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingChoice {
    Save,
    Discard,
    Cancel,
}

/// Column used to sort the task table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortKey {
    Name,
    Start,
    Duration,
    Progress,
}

impl SortKey {
    const fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Start => "Start",
            Self::Duration => "Days",
            Self::Progress => "Progress",
        }
    }
}

/// A deferred document mutation collected while drawing widgets.
#[derive(Debug, Clone)]
enum TaskEdit {
    Title(String),
    Name(u64, String),
    Dependencies(u64, Vec<u64>),
    Start(u64, i64),
    Duration(u64, i64),
    Progress(u64, f32),
    Notes(u64, String),
    Resource(u64, String),
    Cost(u64, f64),
    Milestone(u64, bool),
    CalendarStart(Option<Date>),
    SkipWeekends(bool),
}

/// Active bar drag. The whole drag is one history gesture.
#[derive(Debug, Clone, Copy)]
struct DragState {
    task_id: u64,
    pointer_x: f32,
    original_start: i64,
}

/// The ProjectCraft application state.
pub struct ProjectApp {
    shell: AppShell,
    host: DocumentHost<Document>,
    selected: Option<u64>,
    title_buffer: String,
    name_buffers: BTreeMap<u64, String>,
    dep_buffers: BTreeMap<u64, String>,
    notes_buffers: BTreeMap<u64, String>,
    resource_buffers: BTreeMap<u64, String>,
    schedule_error: Option<String>,
    pending: Option<PendingAction>,
    zoom: Zoom,
    sort: SortKey,
    drag: Option<DragState>,
}

impl ProjectApp {
    /// Build the application, opening the startup project when requested.
    pub fn new(context: &eframe::CreationContext<'_>, options: CliOptions) -> Self {
        let identity = AppIdentity {
            id: "projectcraft",
            name: "ProjectCraft",
            version: APP_VERSION,
            tagline: "Project planning with dependencies, CPM and Gantt",
            repository: Some("https://github.com/emilio-n8n/projectcraft"),
        };
        let shell = AppShell::new(identity);
        let document = if options.sample {
            Document::sample()
        } else {
            Document::default()
        };
        let mut app = Self {
            shell,
            host: DocumentHost::new(document, FORMAT, SCHEMA_VERSION, APP_VERSION),
            selected: None,
            title_buffer: String::new(),
            name_buffers: BTreeMap::new(),
            dep_buffers: BTreeMap::new(),
            notes_buffers: BTreeMap::new(),
            resource_buffers: BTreeMap::new(),
            schedule_error: None,
            pending: None,
            zoom: Zoom::Day,
            sort: SortKey::Start,
            drag: None,
        };
        app.shell.install_theme(&context.egui_ctx);
        app.refresh_buffers();
        if let Some(path) = options.open {
            app.load_path(path);
        }
        app
    }

    fn refresh_buffers(&mut self) {
        let document = self.host.document();
        self.title_buffer = document.title.clone();
        self.name_buffers.clear();
        self.dep_buffers.clear();
        self.notes_buffers.clear();
        self.resource_buffers.clear();
        for task in &document.tasks {
            self.name_buffers.insert(task.id, task.name.clone());
            self.dep_buffers
                .insert(task.id, format_dependencies(&task.dependencies));
            self.notes_buffers.insert(task.id, task.notes.clone());
            self.resource_buffers.insert(task.id, task.resource.clone());
        }
    }

    fn load_path(&mut self, path: PathBuf) {
        match DocumentHost::from_file(path.clone(), FORMAT, SCHEMA_VERSION, APP_VERSION) {
            Ok(host) => {
                self.host = host;
                self.selected = None;
                self.refresh_buffers();
                self.shell.set_status(format!("Opened {}", path.display()));
            }
            Err(error) => {
                self.shell
                    .notify(Tone::Error, "Could not open project", error.to_string());
            }
        }
    }

    fn new_document(&mut self) {
        self.host.reset(Document::default());
        self.selected = None;
        self.refresh_buffers();
        self.shell.set_status("New document");
    }

    fn save(&mut self) -> bool {
        if self.host.path().is_none() {
            return self.save_as();
        }
        match self.host.save() {
            Ok(path) => {
                self.shell.set_status(format!("Saved {}", path.display()));
                true
            }
            Err(error) => {
                self.shell
                    .notify(Tone::Error, "Could not save project", error.to_string());
                false
            }
        }
    }

    fn save_as(&mut self) -> bool {
        let default_name = format!("{}.json", slugify(&self.host.document().title));
        let Some(path) = dialogs::save_project("Save project", &default_name, "json") else {
            return false;
        };
        match self.host.save_as(path) {
            Ok(path) => {
                self.shell.set_status(format!("Saved {}", path.display()));
                true
            }
            Err(error) => {
                self.shell
                    .notify(Tone::Error, "Could not save project", error.to_string());
                false
            }
        }
    }

    fn import_csv(&mut self) {
        let Some(path) = dialogs::open_project("Import tasks from CSV", &["csv"]) else {
            return;
        };
        match import_csv(&path) {
            Ok(document) => {
                let count = document.tasks.len();
                self.host.apply(|current| *current = document);
                self.selected = None;
                self.refresh_buffers();
                self.shell.set_status(format!("Imported {count} tasks"));
            }
            Err(error) => {
                self.shell.notify(Tone::Error, "CSV import failed", error);
            }
        }
    }

    fn export_csv(&mut self) {
        let result = match schedule(self.host.document()) {
            Ok(result) => result,
            Err(error) => {
                self.shell.notify(
                    Tone::Error,
                    "Export failed",
                    format!("Fix the plan: {error}"),
                );
                return;
            }
        };
        let default_name = format!("{}.csv", slugify(&self.host.document().title));
        let Some(path) = dialogs::save_project("Export tasks as CSV", &default_name, "csv") else {
            return;
        };
        match write_csv(&path, self.host.document(), &result) {
            Ok(()) => self
                .shell
                .set_status(format!("Exported {}", path.display())),
            Err(error) => self.shell.notify(Tone::Error, "Export failed", error),
        }
    }

    fn export_pdf(&mut self) {
        let result = match schedule(self.host.document()) {
            Ok(result) => result,
            Err(error) => {
                self.shell.notify(
                    Tone::Error,
                    "Export failed",
                    format!("Fix the plan: {error}"),
                );
                return;
            }
        };
        let default_name = format!("{}-gantt.pdf", slugify(&self.host.document().title));
        let Some(path) = dialogs::save_project("Export Gantt as PDF", &default_name, "pdf") else {
            return;
        };
        match export_pdf(&path, self.host.document(), &result) {
            Ok(()) => self
                .shell
                .set_status(format!("Exported {}", path.display())),
            Err(error) => self.shell.notify(Tone::Error, "Export failed", error),
        }
    }

    fn request_new(&mut self) {
        if self.host.is_dirty() {
            self.pending = Some(PendingAction::New);
        } else {
            self.new_document();
        }
    }

    fn request_open(&mut self) {
        let Some(path) = dialogs::open_project("Open project", &["json"]) else {
            return;
        };
        if self.host.is_dirty() {
            self.pending = Some(PendingAction::Open(path));
        } else {
            self.load_path(path);
        }
    }

    fn perform(&mut self, action: PendingAction) {
        match action {
            PendingAction::New => self.new_document(),
            PendingAction::Open(path) => self.load_path(path),
        }
    }

    fn resolve_pending(&mut self, choice: PendingChoice) {
        let Some(action) = self.pending.take() else {
            return;
        };
        match choice {
            PendingChoice::Cancel => {}
            PendingChoice::Discard => self.perform(action),
            PendingChoice::Save => {
                if self.save() {
                    self.perform(action);
                } else {
                    self.pending = Some(action);
                }
            }
        }
    }

    fn clamp_selection(&mut self) {
        if let Some(id) = self.selected
            && self.host.document().task(id).is_none()
        {
            self.selected = None;
        }
        self.refresh_buffers();
    }

    fn add_task(&mut self) {
        let mut new_id = 0;
        if self.host.apply(|document| new_id = document.add_task()) {
            self.selected = Some(new_id);
            self.refresh_buffers();
            self.shell.set_status("Task added");
        }
    }

    fn add_milestone(&mut self) {
        let after = self.selected;
        let mut new_id = 0;
        if self
            .host
            .apply(|document| new_id = document.add_milestone(after))
        {
            self.selected = Some(new_id);
            self.refresh_buffers();
            self.shell.set_status("Milestone added");
        }
    }

    fn duplicate_selected(&mut self) {
        let Some(id) = self.selected else {
            return;
        };
        let mut copy_id = None;
        if self
            .host
            .apply(|document| copy_id = document.duplicate_task(id))
        {
            self.selected = copy_id;
            self.refresh_buffers();
            self.shell.set_status("Task duplicated");
        }
    }

    fn move_selected(&mut self, delta: isize) {
        let Some(id) = self.selected else {
            return;
        };
        self.host.apply(|document| {
            document.move_task(id, delta);
        });
    }

    fn delete_task(&mut self, id: u64) {
        if self.host.apply(|document| {
            document.remove_task(id);
        }) {
            if self.selected == Some(id) {
                self.selected = None;
            }
            self.refresh_buffers();
            self.shell.set_status("Task deleted");
        }
    }

    /// Apply a queued edit to the document through the undo history.
    fn apply_edit(&mut self, edit: TaskEdit) {
        match edit {
            TaskEdit::Title(value) => {
                self.host.apply(|document| document.title = value);
            }
            TaskEdit::Name(id, value) => {
                self.host.apply(|document| {
                    if let Some(task) = document.task_mut(id) {
                        task.name = value;
                    }
                });
            }
            TaskEdit::Dependencies(id, value) => {
                self.host.apply(|document| {
                    if let Some(task) = document.task_mut(id) {
                        task.dependencies = value;
                    }
                });
            }
            TaskEdit::Start(id, value) => {
                self.host.apply(|document| {
                    if let Some(task) = document.task_mut(id) {
                        task.start_day = value;
                    }
                });
            }
            TaskEdit::Duration(id, value) => {
                self.host.apply(|document| {
                    if let Some(task) = document.task_mut(id) {
                        task.duration_days = value;
                    }
                });
            }
            TaskEdit::Progress(id, value) => {
                self.host.apply(|document| {
                    if let Some(task) = document.task_mut(id) {
                        task.progress = value;
                    }
                });
            }
            TaskEdit::Notes(id, value) => {
                self.host.apply(|document| {
                    if let Some(task) = document.task_mut(id) {
                        task.notes = value;
                    }
                });
            }
            TaskEdit::Resource(id, value) => {
                self.host.apply(|document| {
                    if let Some(task) = document.task_mut(id) {
                        task.resource = value;
                    }
                });
            }
            TaskEdit::Cost(id, value) => {
                self.host.apply(|document| {
                    if let Some(task) = document.task_mut(id) {
                        task.cost = value;
                    }
                });
            }
            TaskEdit::Milestone(id, value) => {
                self.host.apply(|document| {
                    if let Some(task) = document.task_mut(id) {
                        task.milestone = value;
                        if value {
                            task.duration_days = 0;
                        } else if task.duration_days == 0 {
                            task.duration_days = 1;
                        }
                    }
                });
            }
            TaskEdit::CalendarStart(value) => {
                self.host
                    .apply(|document| document.calendar.start_date = value);
            }
            TaskEdit::SkipWeekends(value) => {
                self.host
                    .apply(|document| document.calendar.skip_weekends = value);
            }
        }
    }

    fn handle_action(&mut self, action: Action) {
        match action {
            Action::New => self.request_new(),
            Action::Open => self.request_open(),
            Action::Save => {
                self.save();
            }
            Action::SaveAs => {
                self.save_as();
            }
            Action::ImportCsv => self.import_csv(),
            Action::ExportCsv => self.export_csv(),
            Action::ExportPdf => self.export_pdf(),
            Action::Undo => {
                if self.host.undo() {
                    self.clamp_selection();
                    self.shell.set_status("Undo");
                }
            }
            Action::Redo => {
                if self.host.redo() {
                    self.clamp_selection();
                    self.shell.set_status("Redo");
                }
            }
            Action::About => self.shell.open_about(),
            Action::Settings => self.shell.open_settings(),
        }
    }

    fn handle_shortcuts(&mut self, ui: &egui::Ui) {
        if ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        let action = ui.input_mut(|input| {
            if input.consume_key(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::S,
            ) {
                Some(Action::SaveAs)
            } else if input.consume_key(egui::Modifiers::COMMAND, egui::Key::S) {
                Some(Action::Save)
            } else if input.consume_key(egui::Modifiers::COMMAND, egui::Key::O) {
                Some(Action::Open)
            } else if input.consume_key(egui::Modifiers::COMMAND, egui::Key::N) {
                Some(Action::New)
            } else if input.consume_key(egui::Modifiers::COMMAND, egui::Key::Z) {
                Some(Action::Undo)
            } else if input.consume_key(egui::Modifiers::COMMAND, egui::Key::Y) {
                Some(Action::Redo)
            } else {
                None
            }
        });
        if let Some(action) = action {
            self.handle_action(action);
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let title = self.host.display_name();
        let dirty = self.host.is_dirty();
        let can_undo = self.host.can_undo();
        let can_redo = self.host.can_redo();
        let mut action = None;
        self.shell.top_bar(ui, &title, dirty, |ui| {
            ui.menu_button("File", |ui| {
                for (label, choice) in [
                    ("New", Action::New),
                    ("Open...", Action::Open),
                    ("Save", Action::Save),
                    ("Save As...", Action::SaveAs),
                    ("Import CSV...", Action::ImportCsv),
                    ("Export CSV...", Action::ExportCsv),
                    ("Export Gantt PDF...", Action::ExportPdf),
                ] {
                    if ui.button(label).clicked() {
                        action = Some(choice);
                        ui.close();
                    }
                }
            });
            ui.menu_button("Edit", |ui| {
                if ui
                    .add_enabled(can_undo, egui::Button::new("Undo"))
                    .clicked()
                {
                    action = Some(Action::Undo);
                    ui.close();
                }
                if ui
                    .add_enabled(can_redo, egui::Button::new("Redo"))
                    .clicked()
                {
                    action = Some(Action::Redo);
                    ui.close();
                }
            });
            if ui.button("About").clicked() {
                action = Some(Action::About);
            }
            if ui.button("Settings").clicked() {
                action = Some(Action::Settings);
            }
        });
        if let Some(action) = action {
            self.handle_action(action);
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let error = self.schedule_error.clone();
        self.shell.status_bar_with(ui, |ui| {
            if let Some(error) = error {
                ui.label(RichText::new(error).color(theme::ERROR));
            }
        });
    }

    /// Tasks cloned and ordered by the selected sort key.
    fn sorted_tasks(&self) -> Vec<crate::document::Task> {
        let mut tasks = self.host.document().tasks.clone();
        match self.sort {
            SortKey::Name => tasks.sort_by(|a, b| a.name.cmp(&b.name)),
            SortKey::Start => tasks.sort_by_key(|task| (task.start_day, task.id)),
            SortKey::Duration => tasks.sort_by_key(|task| (task.duration_days, task.id)),
            SortKey::Progress => tasks.sort_by(|a, b| a.progress.total_cmp(&b.progress)),
        }
        tasks
    }

    fn task_table(
        &mut self,
        ui: &mut egui::Ui,
        edits: &mut Vec<TaskEdit>,
        messages: &mut Vec<String>,
    ) {
        let tasks = self.sorted_tasks();
        let mut to_delete = Vec::new();
        {
            let name_buffers = &mut self.name_buffers;
            let dep_buffers = &mut self.dep_buffers;
            let selected = &mut self.selected;

            ScrollArea::horizontal()
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.set_min_width(TABLE_WIDTH);

                    ui.horizontal(|ui| {
                        ui.add_sized([ID_WIDTH, CELL_HEIGHT], egui::Label::new("ID"));
                        for (key, width) in [
                            (SortKey::Name, NAME_WIDTH),
                            (SortKey::Start, START_WIDTH),
                            (SortKey::Duration, DURATION_WIDTH),
                        ] {
                            let label = if self.sort == key {
                                format!("{} ▾", key.label())
                            } else {
                                key.label().to_owned()
                            };
                            if ui
                                .add_sized([width, CELL_HEIGHT], egui::Button::new(label))
                                .clicked()
                            {
                                self.sort = key;
                            }
                        }
                        ui.add_sized(
                            [DEPENDENCIES_WIDTH, CELL_HEIGHT],
                            egui::Label::new("Depends on"),
                        );
                        if ui
                            .add_sized(
                                [PROGRESS_WIDTH, CELL_HEIGHT],
                                egui::Button::new(SortKey::Progress.label()),
                            )
                            .clicked()
                        {
                            self.sort = SortKey::Progress;
                        }
                        ui.add_sized([DELETE_WIDTH, CELL_HEIGHT], egui::Label::new(""));
                    });
                    ui.separator();

                    for task in &tasks {
                        ui.horizontal(|ui| {
                            let is_selected = *selected == Some(task.id);
                            let label = if task.milestone {
                                format!("◆ {}", task.id)
                            } else {
                                task.id.to_string()
                            };
                            let id_button = egui::Button::new(label).fill(if is_selected {
                                theme::ACCENT
                            } else {
                                theme::PANEL
                            });
                            if ui.add_sized([ID_WIDTH, CELL_HEIGHT], id_button).clicked() {
                                *selected = Some(task.id);
                            }

                            let name = name_buffers
                                .entry(task.id)
                                .or_insert_with(|| task.name.clone());
                            let response =
                                ui.add_sized([NAME_WIDTH, CELL_HEIGHT], TextEdit::singleline(name));
                            if response.lost_focus() && name != &task.name {
                                edits.push(TaskEdit::Name(task.id, name.clone()));
                            }

                            let mut start = task.start_day;
                            if ui
                                .add_sized([START_WIDTH, CELL_HEIGHT], DragValue::new(&mut start))
                                .changed()
                            {
                                edits.push(TaskEdit::Start(task.id, start));
                            }

                            let mut duration = task.duration_days;
                            if ui
                                .add_sized(
                                    [DURATION_WIDTH, CELL_HEIGHT],
                                    DragValue::new(&mut duration).range(0..=3650),
                                )
                                .changed()
                            {
                                edits.push(TaskEdit::Duration(task.id, duration));
                            }

                            let dependencies = dep_buffers
                                .entry(task.id)
                                .or_insert_with(|| format_dependencies(&task.dependencies));
                            let response = ui.add_sized(
                                [DEPENDENCIES_WIDTH, CELL_HEIGHT],
                                TextEdit::singleline(dependencies).hint_text("1,2"),
                            );
                            if response.lost_focus() {
                                let raw = dependencies.clone();
                                match parse_dependencies(&raw) {
                                    Ok(ids) => {
                                        if ids != task.dependencies {
                                            edits.push(TaskEdit::Dependencies(task.id, ids));
                                        }
                                    }
                                    Err(error) => {
                                        messages.push(error);
                                        *dependencies = format_dependencies(&task.dependencies);
                                    }
                                }
                            }

                            let mut percent =
                                (task.progress.clamp(0.0, 1.0) * 100.0).round() as i32;
                            ui.allocate_ui_with_layout(
                                vec2(PROGRESS_WIDTH, CELL_HEIGHT),
                                Layout::left_to_right(Align::Center),
                                |ui| {
                                    if ui
                                        .add_sized(
                                            [72.0, CELL_HEIGHT],
                                            DragValue::new(&mut percent).range(0..=100).suffix("%"),
                                        )
                                        .changed()
                                    {
                                        edits.push(TaskEdit::Progress(
                                            task.id,
                                            percent as f32 / 100.0,
                                        ));
                                    }
                                    ui.add_sized(
                                        [30.0, 12.0],
                                        egui::ProgressBar::new(task.progress.clamp(0.0, 1.0)),
                                    );
                                },
                            );

                            if ui
                                .add_sized([DELETE_WIDTH, CELL_HEIGHT], egui::Button::new("x"))
                                .clicked()
                            {
                                to_delete.push(task.id);
                            }
                        });
                    }
                });
        }
        for id in to_delete {
            self.delete_task(id);
        }
    }

    /// Inspector for the selected task: milestone, notes, resource, cost.
    fn details_panel(&mut self, ui: &mut egui::Ui, result: &Schedule, edits: &mut Vec<TaskEdit>) {
        let Some(id) = self.selected else {
            ui.label(RichText::new("Select a task to edit its details.").color(theme::FAINT));
            return;
        };
        let Some(task) = self.host.document().task(id).cloned() else {
            return;
        };
        let timing = result.task(id).cloned();

        ui.separator();
        ui.horizontal(|ui| {
            ui.heading("Task details");
            if ui.small_button("Duplicate").clicked() {
                self.duplicate_selected();
            }
            if ui.small_button("Move up").clicked() {
                self.move_selected(-1);
            }
            if ui.small_button("Move down").clicked() {
                self.move_selected(1);
            }
        });

        let mut milestone = task.milestone;
        if ui
            .checkbox(&mut milestone, "Milestone (zero-duration marker)")
            .changed()
        {
            edits.push(TaskEdit::Milestone(id, milestone));
        }

        ui.horizontal(|ui| {
            ui.label("Resource");
            let buffer = self
                .resource_buffers
                .entry(id)
                .or_insert_with(|| task.resource.clone());
            let response = ui.add(TextEdit::singleline(buffer).desired_width(180.0));
            if response.lost_focus() && buffer != &task.resource {
                edits.push(TaskEdit::Resource(id, buffer.clone()));
            }
            ui.label("Cost");
            let mut cost = task.cost;
            if ui
                .add(
                    DragValue::new(&mut cost)
                        .range(0.0..=1_000_000.0)
                        .prefix("¤"),
                )
                .changed()
            {
                edits.push(TaskEdit::Cost(id, cost));
            }
        });

        ui.horizontal(|ui| {
            ui.label("Notes");
            let buffer = self
                .notes_buffers
                .entry(id)
                .or_insert_with(|| task.notes.clone());
            let response = ui.add(
                TextEdit::singleline(buffer)
                    .desired_width(420.0)
                    .hint_text("Optional notes"),
            );
            if response.lost_focus() && buffer != &task.notes {
                edits.push(TaskEdit::Notes(id, buffer.clone()));
            }
        });

        if let Some(timing) = timing {
            let calendar = &self.host.document().calendar;
            let earliest = calendar.label_for_day(timing.earliest_start);
            let finish = calendar.label_for_day(timing.earliest_finish);
            ui.label(
                RichText::new(format!(
                    "Earliest {earliest} → {finish}  •  float {} day(s)  •  {}",
                    timing.total_float,
                    if timing.is_critical() {
                        "critical"
                    } else {
                        "flexible"
                    }
                ))
                .color(theme::MUTED),
            );
        }
    }

    /// Project-wide calendar settings.
    fn calendar_panel(&mut self, ui: &mut egui::Ui, edits: &mut Vec<TaskEdit>) {
        let calendar = self.host.document().calendar.clone();
        ui.horizontal(|ui| {
            ui.label("Start date");
            let mut enabled = calendar.start_date.is_some();
            if ui.checkbox(&mut enabled, "Real dates").changed() {
                let date = enabled.then(|| Date::new(2026, 11, 23).expect("valid date"));
                edits.push(TaskEdit::CalendarStart(date));
            }
            if let Some(date) = calendar.start_date {
                let mut year = date.year;
                let mut month = date.month as i32;
                let mut day = date.day as i32;
                if ui
                    .add(DragValue::new(&mut year).range(1970..=2100).prefix("Y "))
                    .changed()
                    && let Some(valid) = Date::new(year, date.month, date.day)
                {
                    edits.push(TaskEdit::CalendarStart(Some(valid)));
                }
                if ui
                    .add(DragValue::new(&mut month).range(1..=12).prefix("M "))
                    .changed()
                    && let Some(valid) = Date::new(date.year, month as u32, date.day)
                {
                    edits.push(TaskEdit::CalendarStart(Some(valid)));
                }
                if ui
                    .add(DragValue::new(&mut day).range(1..=31).prefix("D "))
                    .changed()
                    && let Some(valid) = Date::new(date.year, date.month, day as u32)
                {
                    edits.push(TaskEdit::CalendarStart(Some(valid)));
                }
            }
            let mut skip = calendar.skip_weekends;
            if ui.checkbox(&mut skip, "Skip weekends").changed() {
                edits.push(TaskEdit::SkipWeekends(skip));
            }
        });
    }

    fn gantt(&mut self, ui: &mut egui::Ui, result: &Schedule) {
        let tasks = self.host.document().tasks.clone();
        let days = result.project_duration.max(1);
        let available_width = ui.available_width();
        let day_width = self.zoom.day_width(days, available_width);
        let content_width = LABEL_WIDTH + days as f32 * day_width + 24.0;
        let content_height = HEADER_HEIGHT + tasks.len() as f32 * ROW_HEIGHT + 12.0;
        let font = FontId::new(11.0, FontFamily::Proportional);
        let small_font = FontId::new(9.5, FontFamily::Proportional);
        let calendar = self.host.document().calendar.clone();

        ui.horizontal(|ui| {
            ui.label("Zoom");
            for zoom in Zoom::all() {
                if ui
                    .selectable_label(self.zoom == zoom, zoom.label())
                    .clicked()
                {
                    self.zoom = zoom;
                }
            }
            ui.label(
                RichText::new("Click a bar to select • drag to reschedule").color(theme::FAINT),
            );
        });

        ScrollArea::both()
            .auto_shrink([false, false])
            .max_height(360.0)
            .show(ui, |ui| {
                let (rect, response) = ui.allocate_exact_size(
                    Vec2::new(content_width, content_height),
                    Sense::click_and_drag(),
                );
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, CornerRadius::ZERO, theme::SUNKEN);

                // Alternating row backgrounds and row hit-test rectangles.
                for row in 0..tasks.len() {
                    if row % 2 == 0 {
                        continue;
                    }
                    let top = rect.min.y + HEADER_HEIGHT + row as f32 * ROW_HEIGHT;
                    let row_rect =
                        Rect::from_min_size(pos2(rect.min.x, top), vec2(content_width, ROW_HEIGHT));
                    painter.rect_filled(row_rect, CornerRadius::ZERO, theme::PANEL);
                }

                let header = Rect::from_min_size(rect.min, vec2(content_width, HEADER_HEIGHT));
                painter.rect_filled(header, CornerRadius::ZERO, theme::RAISED);
                let step = self.zoom.ruler_step(days).max(1);
                for day in (0..=days).step_by(step) {
                    let x = day_x(day, rect.min.x + LABEL_WIDTH, day_width);
                    painter.line_segment(
                        [pos2(x, rect.min.y), pos2(x, rect.max.y)],
                        Stroke::new(1.0, theme::BORDER),
                    );
                    painter.text(
                        pos2(x + 3.0, rect.min.y + HEADER_HEIGHT / 2.0),
                        Align2::LEFT_CENTER,
                        calendar.label_for_day(day),
                        small_font.clone(),
                        theme::MUTED,
                    );
                }
                painter.text(
                    pos2(rect.min.x + 8.0, rect.min.y + HEADER_HEIGHT / 2.0),
                    Align2::LEFT_CENTER,
                    "Task / date",
                    small_font,
                    theme::FAINT,
                );

                // Task labels and row selection.
                for (row, task) in tasks.iter().enumerate() {
                    let center_y =
                        rect.min.y + HEADER_HEIGHT + row as f32 * ROW_HEIGHT + ROW_HEIGHT / 2.0;
                    painter.text(
                        pos2(rect.min.x + 8.0, center_y),
                        Align2::LEFT_CENTER,
                        if self.selected == Some(task.id) {
                            format!("▸ {}", task.name)
                        } else {
                            task.name.clone()
                        },
                        font.clone(),
                        if result
                            .task(task.id)
                            .is_some_and(|timing| timing.is_critical())
                        {
                            theme::ACCENT_LIGHT
                        } else {
                            theme::TEXT
                        },
                    );
                }

                // Dependency arrows, drawn behind the bars.
                for (row, task) in tasks.iter().enumerate() {
                    let Some(timing) = result.task(task.id) else {
                        continue;
                    };
                    let to_y =
                        rect.min.y + HEADER_HEIGHT + row as f32 * ROW_HEIGHT + ROW_HEIGHT / 2.0;
                    let to_x = day_x(timing.earliest_start, rect.min.x + LABEL_WIDTH, day_width);
                    for dependency in &task.dependencies {
                        let Some(parent_index) =
                            tasks.iter().position(|item| item.id == *dependency)
                        else {
                            continue;
                        };
                        let Some(parent) = result.task(*dependency) else {
                            continue;
                        };
                        let from_y = rect.min.y
                            + HEADER_HEIGHT
                            + parent_index as f32 * ROW_HEIGHT
                            + ROW_HEIGHT / 2.0;
                        let from_x =
                            day_x(parent.earliest_finish, rect.min.x + LABEL_WIDTH, day_width);
                        let points = arrow_points((from_x, from_y), (to_x, to_y));
                        for pair in points.windows(2) {
                            painter.line_segment(
                                [pos2(pair[0].0, pair[0].1), pos2(pair[1].0, pair[1].1)],
                                Stroke::new(1.0, theme::BORDER_STRONG),
                            );
                        }
                        painter.arrow(
                            pos2(to_x - 5.0, to_y),
                            vec2(5.0, 0.0),
                            Stroke::new(1.0, theme::BORDER_STRONG),
                        );
                    }
                }

                // Bars, milestones and drag interaction.
                for (row, task) in tasks.iter().enumerate() {
                    let Some(timing) = result.task(task.id) else {
                        continue;
                    };
                    let center_y =
                        rect.min.y + HEADER_HEIGHT + row as f32 * ROW_HEIGHT + ROW_HEIGHT / 2.0;
                    let (left, top, right, bottom) = bar_rect(
                        timing.earliest_start,
                        timing.earliest_finish,
                        rect.min.x + LABEL_WIDTH,
                        center_y,
                        day_width,
                    );
                    let bar = Rect::from_min_max(pos2(left, top), pos2(right, bottom));

                    let color = if timing.is_critical() {
                        theme::ACCENT
                    } else {
                        theme::INFO
                    };
                    if task.milestone {
                        let mid = (left + right) / 2.0;
                        let half = 7.0;
                        let diamond = vec![
                            pos2(mid, center_y - half),
                            pos2(mid + half, center_y),
                            pos2(mid, center_y + half),
                            pos2(mid - half, center_y),
                        ];
                        painter.add(Shape::convex_polygon(diamond, color, Stroke::NONE));
                    } else {
                        painter.rect_filled(bar, CornerRadius::same(4), color);
                        if task.progress > 0.0 {
                            let width = bar.width() * task.progress.clamp(0.0, 1.0);
                            let done = Rect::from_min_size(bar.min, vec2(width, bar.height()));
                            painter.rect_filled(done, CornerRadius::same(4), theme::SUCCESS);
                        }
                    }
                    if !timing.is_critical() && timing.total_float > 0 {
                        let slack_end =
                            day_x(timing.latest_finish, rect.min.x + LABEL_WIDTH, day_width);
                        painter.line_segment(
                            [pos2(right, center_y), pos2(slack_end, center_y)],
                            Stroke::new(2.0, theme::FAINT),
                        );
                    }
                }

                // Start or continue a reschedule gesture from the bar drag.
                if let Some(pointer) = response.interact_pointer_pos() {
                    if response.drag_started() {
                        let relative_y = pointer.y - rect.min.y;
                        if let Some(row) = row_at(relative_y)
                            && let Some(task) = tasks.get(row)
                        {
                            self.host.history_mut().begin_gesture();
                            self.drag = Some(DragState {
                                task_id: task.id,
                                pointer_x: pointer.x,
                                original_start: task.start_day,
                            });
                            self.selected = Some(task.id);
                        }
                    }
                    if let Some(drag) = self.drag
                        && response.dragged()
                    {
                        let delta = drag_delta_days(drag.pointer_x, pointer.x, day_width);
                        if let Some(task) =
                            self.host.history_mut().current_mut().task_mut(drag.task_id)
                        {
                            task.start_day = drag.original_start + delta;
                        }
                    }
                }
                if response.drag_stopped() && self.drag.take().is_some() {
                    self.host.history_mut().end_gesture();
                    self.shell.set_status("Bar rescheduled");
                }
            });
    }

    fn central_panel(&mut self, ui: &mut egui::Ui) {
        let mut edits = Vec::new();
        let mut messages = Vec::new();

        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Task plan");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::toolbar_button(ui, "PDF", "Export the Gantt as PDF").clicked() {
                        self.export_pdf();
                    }
                    if widgets::toolbar_button(ui, "CSV", "Export the task table").clicked() {
                        self.export_csv();
                    }
                    if widgets::toolbar_button(ui, "Milestone", "Add a milestone").clicked() {
                        self.add_milestone();
                    }
                    if widgets::toolbar_button(ui, "Add task", "Append a new task").clicked() {
                        self.add_task();
                    }
                });
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Project");
                let response =
                    ui.add(TextEdit::singleline(&mut self.title_buffer).hint_text("Project title"));
                if response.lost_focus() && self.title_buffer != self.host.document().title {
                    edits.push(TaskEdit::Title(self.title_buffer.clone()));
                }
            });
            self.calendar_panel(ui, &mut edits);
            ui.separator();

            match schedule(self.host.document()) {
                Ok(result) => {
                    self.schedule_error = None;
                    let critical = result.critical_task_ids().len();
                    let cost = self.host.document().total_cost();
                    ui.label(
                        RichText::new(format!(
                            "Duration: {} days  •  Critical: {critical}  •  Cost: {cost:.2}",
                            result.project_duration
                        ))
                        .color(theme::MUTED),
                    );
                    if self.host.document().tasks.is_empty() {
                        widgets::empty_state(
                            ui,
                            "No tasks yet",
                            "Add a task or a milestone to start planning.",
                        );
                    } else {
                        self.task_table(ui, &mut edits, &mut messages);
                        self.details_panel(ui, &result, &mut edits);
                        ui.separator();
                        self.gantt(ui, &result);
                    }
                }
                Err(error) => {
                    self.schedule_error = Some(error.clone());
                    ui.colored_label(theme::ERROR, format!("Cannot schedule: {error}"));
                    self.task_table(ui, &mut edits, &mut messages);
                }
            }
        });

        for edit in edits {
            self.apply_edit(edit);
        }
        for message in messages {
            self.shell.set_status(message);
        }
    }

    fn unsaved_window(&mut self, ctx: &egui::Context) {
        if self.pending.is_none() {
            return;
        }
        let mut open = true;
        let mut choice = None;
        egui::Window::new("Unsaved changes")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(380.0)
            .show(ctx, |ui| {
                ui.label("The current project has unsaved changes.");
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Save and continue").clicked() {
                        choice = Some(PendingChoice::Save);
                    }
                    if ui.button("Discard").clicked() {
                        choice = Some(PendingChoice::Discard);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(PendingChoice::Cancel);
                    }
                });
            });
        if let Some(choice) = choice {
            self.resolve_pending(choice);
        } else if !open {
            self.pending = None;
        }
    }
}

impl eframe::App for ProjectApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.handle_shortcuts(ui);
        self.top_bar(ui);
        self.status_bar(ui);
        self.central_panel(ui);
        self.unsaved_window(ui.ctx());
        self.shell.show_windows(ui.ctx());
        self.shell
            .update_title(ui.ctx(), &self.host.display_name(), self.host.is_dirty());
    }
}

fn slugify(title: &str) -> String {
    let mut slug = String::new();
    for character in title.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "untitled".to_owned()
    } else {
        slug.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dependency_input_accepts_comma_separated_ids() {
        assert_eq!(parse_dependencies("1,2").expect("parse"), vec![1, 2]);
    }

    #[test]
    fn slugify_produces_a_safe_file_name() {
        assert_eq!(slugify("Sample project!"), "sample-project");
        assert_eq!(slugify("  "), "untitled");
    }
}
