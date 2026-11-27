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
use crate::document::{
    Document, format_dependencies, parse_dependencies, validate_dependency_change,
};
use crate::gantt::{
    HEADER_HEIGHT, LABEL_WIDTH, ROW_HEIGHT, Zoom, arrow_points, bar_rect, day_x, drag_delta_days,
    drag_target_start,
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
    AddTask,
    AddMilestone,
    DuplicateSelected,
    MoveUp,
    MoveDown,
    About,
    Settings,
}

#[derive(Debug, Clone)]
enum PendingAction {
    New,
    Open(PathBuf),
    Exit,
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
    Notes(u64, String),
    Resource(u64, String),
    Milestone(u64, bool),
    CalendarStart(Option<Date>),
    SkipWeekends(bool),
}

/// Active bar drag. The whole drag is one history gesture.
#[derive(Debug, Clone, Copy)]
struct DragState {
    task_id: u64,
    pointer_x: f32,
    day_width: f32,
    original_earliest_start: i64,
    original_release: i64,
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
    value_gesture: bool,
    allow_close_once: bool,
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
            value_gesture: false,
            allow_close_once: false,
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

    fn finish_drag(&mut self) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        self.host.history_mut().end_gesture();
        if self
            .host
            .document()
            .task(drag.task_id)
            .is_some_and(|task| task.start_day != drag.original_release)
        {
            self.shell.set_status("Bar rescheduled");
        }
    }

    fn finish_value_gesture(&mut self) {
        finish_value_gesture(&mut self.host, &mut self.value_gesture);
    }

    fn load_path(&mut self, path: PathBuf) {
        self.finish_drag();
        self.finish_value_gesture();
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
        self.finish_drag();
        self.finish_value_gesture();
        self.host.reset(Document::default());
        self.selected = None;
        self.refresh_buffers();
        self.shell.set_status("New document");
    }

    fn save(&mut self) -> bool {
        self.finish_drag();
        self.finish_value_gesture();
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
        self.finish_drag();
        self.finish_value_gesture();
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
                self.finish_drag();
                self.finish_value_gesture();
                replace_with_import(&mut self.host, document);
                self.selected = None;
                self.refresh_buffers();
                self.shell
                    .set_status(format!("Imported {count} tasks as a new unsaved project"));
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

    fn perform(&mut self, action: PendingAction, ctx: &egui::Context) {
        match action {
            PendingAction::New => self.new_document(),
            PendingAction::Open(path) => self.load_path(path),
            PendingAction::Exit => {
                self.allow_close_once = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn resolve_pending(&mut self, choice: PendingChoice, ctx: &egui::Context) {
        let Some(action) = self.pending.take() else {
            return;
        };
        match choice {
            PendingChoice::Cancel => {}
            PendingChoice::Discard => self.perform(action, ctx),
            PendingChoice::Save => {
                if self.save() {
                    self.perform(action, ctx);
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
        self.finish_drag();
        let mut new_id = 0;
        if self.host.apply(|document| new_id = document.add_task()) {
            self.selected = Some(new_id);
            self.refresh_buffers();
            self.shell.set_status("Task added");
        }
    }

    fn add_milestone(&mut self) {
        self.finish_drag();
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
        self.finish_drag();
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
        self.finish_drag();
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
        self.finish_drag();
        self.finish_value_gesture();
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

    fn flush_text_edits(&mut self) {
        if self.title_buffer != self.host.document().title {
            self.apply_edit(TaskEdit::Title(self.title_buffer.clone()));
        }
        let tasks = self.host.document().tasks.clone();
        let mut message = None;
        for task in tasks {
            if let Some(value) = self.name_buffers.get(&task.id)
                && value != &task.name
            {
                if value.trim().is_empty() {
                    self.name_buffers.insert(task.id, task.name.clone());
                    message = Some("Task name cannot be empty".to_owned());
                } else {
                    self.apply_edit(TaskEdit::Name(task.id, value.clone()));
                }
            }
            if let Some(value) = self.dep_buffers.get(&task.id)
                && value != &format_dependencies(&task.dependencies)
            {
                match parse_dependencies(value).and_then(|ids| {
                    validate_dependency_change(self.host.document(), task.id, &ids).map(|()| ids)
                }) {
                    Ok(ids) if ids != task.dependencies => {
                        self.apply_edit(TaskEdit::Dependencies(task.id, ids));
                    }
                    Ok(_) => {}
                    Err(error) => {
                        self.dep_buffers
                            .insert(task.id, format_dependencies(&task.dependencies));
                        message = Some(error);
                    }
                }
            }
            if let Some(value) = self.notes_buffers.get(&task.id)
                && value != &task.notes
            {
                self.apply_edit(TaskEdit::Notes(task.id, value.clone()));
            }
            if let Some(value) = self.resource_buffers.get(&task.id)
                && value != &task.resource
            {
                self.apply_edit(TaskEdit::Resource(task.id, value.clone()));
            }
        }
        if let Some(message) = message {
            self.shell.set_status(message);
        }
    }

    fn handle_action(&mut self, action: Action) {
        self.finish_drag();
        self.finish_value_gesture();
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
            Action::AddTask => self.add_task(),
            Action::AddMilestone => self.add_milestone(),
            Action::DuplicateSelected => self.duplicate_selected(),
            Action::MoveUp => self.move_selected(-1),
            Action::MoveDown => self.move_selected(1),
            Action::About => self.shell.open_about(),
            Action::Settings => self.shell.open_settings(),
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) -> Option<Action> {
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
        action
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
                .id_salt("task_table")
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
                            let label = sort_header_label(key, self.sort == key);
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
                            let label = task_id_label(task.id, task.milestone);
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
                                if name.trim().is_empty() {
                                    messages.push("Task name cannot be empty".to_owned());
                                    *name = task.name.clone();
                                } else {
                                    edits.push(TaskEdit::Name(task.id, name.clone()));
                                }
                            }

                            let mut start = task.start_day;
                            let response =
                                ui.add_sized([START_WIDTH, CELL_HEIGHT], DragValue::new(&mut start));
                            begin_value_gesture(&mut self.host, &mut self.value_gesture, &response);
                            if response.changed()
                                && let Some(current) =
                                    self.host.history_mut().current_mut().task_mut(task.id)
                            {
                                current.start_day = start;
                            }
                            finish_value_gesture_if(&mut self.host, &mut self.value_gesture, &response);

                            let mut duration = task.duration_days;
                            let response = ui.add_sized(
                                [DURATION_WIDTH, CELL_HEIGHT],
                                DragValue::new(&mut duration).range(0..=3650),
                            );
                            begin_value_gesture(&mut self.host, &mut self.value_gesture, &response);
                            if response.changed() {
                                if task.milestone && duration != 0 {
                                    messages.push(
                                        "Milestones must have zero duration; uncheck Milestone to set days"
                                            .to_owned(),
                                    );
                                } else if let Some(current) =
                                    self.host.history_mut().current_mut().task_mut(task.id)
                                {
                                    current.duration_days = duration;
                                }
                            }
                            finish_value_gesture_if(&mut self.host, &mut self.value_gesture, &response);

                            let dependencies = dep_buffers
                                .entry(task.id)
                                .or_insert_with(|| format_dependencies(&task.dependencies));
                            let response = ui.add_sized(
                                [DEPENDENCIES_WIDTH, CELL_HEIGHT],
                                TextEdit::singleline(dependencies).hint_text("1,2"),
                            );
                            if response.lost_focus() {
                                let raw = dependencies.clone();
                                match parse_dependencies(&raw).and_then(|ids| {
                                    validate_dependency_change(self.host.document(), task.id, &ids)
                                        .map(|()| ids)
                                }) {
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
                                    let response = ui.add_sized(
                                        [72.0, CELL_HEIGHT],
                                        DragValue::new(&mut percent).range(0..=100).suffix("%"),
                                    );
                                    begin_value_gesture(&mut self.host, &mut self.value_gesture, &response);
                                    if response.changed()
                                        && let Some(current) =
                                            self.host.history_mut().current_mut().task_mut(task.id)
                                    {
                                        current.progress = percent as f32 / 100.0;
                                    }
                                    finish_value_gesture_if(&mut self.host, &mut self.value_gesture, &response);
                                    ui.add_sized(
                                        [30.0, 12.0],
                                        egui::ProgressBar::new(task.progress.clamp(0.0, 1.0)),
                                    );
                                },
                            );

                            let delete_response = ui.add_sized(
                                [DELETE_WIDTH, CELL_HEIGHT],
                                egui::Button::new("x"),
                            );
                            if delete_response.on_hover_text("Delete task").clicked() {
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
    fn details_panel(
        &mut self,
        ui: &mut egui::Ui,
        result: &Schedule,
        edits: &mut Vec<TaskEdit>,
        actions: &mut Vec<Action>,
    ) {
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
                actions.push(Action::DuplicateSelected);
            }
            if ui.small_button("Move up").clicked() {
                actions.push(Action::MoveUp);
            }
            if ui.small_button("Move down").clicked() {
                actions.push(Action::MoveDown);
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
            let response = ui.add(
                DragValue::new(&mut cost)
                    .range(0.0..=1_000_000.0)
                    .prefix("¤"),
            );
            begin_value_gesture(&mut self.host, &mut self.value_gesture, &response);
            if response.changed()
                && let Some(current) = self.host.history_mut().current_mut().task_mut(id)
            {
                current.cost = cost;
            }
            finish_value_gesture_if(&mut self.host, &mut self.value_gesture, &response);
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
                    "Earliest {earliest} to {finish}  |  float {} day(s)  |  {}",
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
                let date = enabled.then(Date::today);
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
        let days = result.day_span().max(1);
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
                RichText::new("Click a bar to select; drag it to reschedule.").color(theme::FAINT),
            );
        });

        ScrollArea::both()
            .id_salt("gantt")
            .auto_shrink([false, false])
            .max_height(360.0)
            .show(ui, |ui| {
                let (rect, _response) = ui
                    .allocate_exact_size(Vec2::new(content_width, content_height), Sense::hover());
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
                let step = self.zoom.ruler_step(days).max(1) as i64;
                let tick_count = days / step;
                for tick in 0..=tick_count {
                    let offset = tick * step;
                    let day = result.first_day().saturating_add(offset);
                    let x = day_x(offset, rect.min.x + LABEL_WIDTH, day_width);
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

                let mut label_hits = Vec::with_capacity(tasks.len());
                let mut bar_hits = Vec::with_capacity(tasks.len());

                // Task labels and row selection.
                for (row, task) in tasks.iter().enumerate() {
                    let center_y =
                        rect.min.y + HEADER_HEIGHT + row as f32 * ROW_HEIGHT + ROW_HEIGHT / 2.0;
                    painter.text(
                        pos2(rect.min.x + 8.0, center_y),
                        Align2::LEFT_CENTER,
                        gantt_task_label(&task.name, self.selected == Some(task.id)),
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
                    let label_rect = Rect::from_min_size(
                        pos2(rect.min.x, center_y - ROW_HEIGHT / 2.0),
                        vec2(LABEL_WIDTH, ROW_HEIGHT),
                    );
                    label_hits.push((task.id, label_rect));
                }

                // Dependency arrows, drawn behind the bars.
                for (row, task) in tasks.iter().enumerate() {
                    let Some(timing) = result.task(task.id) else {
                        continue;
                    };
                    let to_y =
                        rect.min.y + HEADER_HEIGHT + row as f32 * ROW_HEIGHT + ROW_HEIGHT / 2.0;
                    let to_x = day_x(
                        result.day_offset(timing.earliest_start),
                        rect.min.x + LABEL_WIDTH,
                        day_width,
                    );
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
                        let from_x = day_x(
                            result.day_offset(parent.earliest_finish),
                            rect.min.x + LABEL_WIDTH,
                            day_width,
                        );
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

                // Bars, milestones and precise interaction regions.
                for (row, task) in tasks.iter().enumerate() {
                    let Some(timing) = result.task(task.id) else {
                        continue;
                    };
                    let center_y =
                        rect.min.y + HEADER_HEIGHT + row as f32 * ROW_HEIGHT + ROW_HEIGHT / 2.0;
                    let (left, top, right, bottom) = bar_rect(
                        result.day_offset(timing.earliest_start),
                        result.day_offset(timing.earliest_finish),
                        rect.min.x + LABEL_WIDTH,
                        center_y,
                        day_width,
                    );
                    let bar = Rect::from_min_max(pos2(left, top), pos2(right, bottom));
                    let selected = self.selected == Some(task.id);

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
                        painter.add(Shape::convex_polygon(
                            diamond,
                            color,
                            if selected {
                                Stroke::new(2.0, theme::ACCENT_LIGHT)
                            } else {
                                Stroke::NONE
                            },
                        ));
                    } else {
                        painter.rect_filled(bar, CornerRadius::same(4), color);
                        if selected {
                            painter.rect_stroke(
                                bar,
                                CornerRadius::same(4),
                                Stroke::new(2.0, theme::ACCENT_LIGHT),
                                egui::StrokeKind::Inside,
                            );
                        }
                        if task.progress > 0.0 {
                            let width = bar.width() * task.progress.clamp(0.0, 1.0);
                            let done = Rect::from_min_size(bar.min, vec2(width, bar.height()));
                            painter.rect_filled(done, CornerRadius::same(4), theme::SUCCESS);
                        }
                    }
                    if !timing.is_critical() && timing.total_float > 0 {
                        let slack_end = day_x(
                            result.day_offset(timing.latest_finish),
                            rect.min.x + LABEL_WIDTH,
                            day_width,
                        );
                        painter.line_segment(
                            [pos2(right, center_y), pos2(slack_end, center_y)],
                            Stroke::new(2.0, theme::FAINT),
                        );
                    }
                    let hit_rect = Rect::from_min_max(
                        pos2(left - 8.0, center_y - ROW_HEIGHT / 2.0),
                        pos2(right + 8.0, center_y + ROW_HEIGHT / 2.0),
                    );
                    bar_hits.push((task.id, hit_rect, timing.earliest_start, task.name.clone()));
                }

                for (task_id, label_rect) in label_hits {
                    let response = ui.interact(
                        label_rect,
                        ui.scope_id().with(("gantt-label", task_id)),
                        Sense::click(),
                    );
                    if response.clicked() {
                        self.selected = Some(task_id);
                    }
                    response.on_hover_cursor(egui::CursorIcon::PointingHand);
                }

                for (task_id, hit_rect, earliest_start, name) in bar_hits {
                    let response = ui.interact(
                        hit_rect,
                        ui.scope_id().with(("gantt-bar", task_id)),
                        Sense::click_and_drag(),
                    );
                    if response.clicked() {
                        self.selected = Some(task_id);
                    }
                    if response.drag_started() {
                        self.finish_drag();
                        self.finish_value_gesture();
                        if let Some(pointer) = response.interact_pointer_pos() {
                            self.host.history_mut().begin_gesture();
                            self.drag = Some(DragState {
                                task_id,
                                pointer_x: pointer.x,
                                day_width,
                                original_earliest_start: earliest_start,
                                original_release: self
                                    .host
                                    .document()
                                    .task(task_id)
                                    .map_or(earliest_start, |task| task.start_day),
                            });
                            self.selected = Some(task_id);
                        }
                    }
                    if self.drag.is_some_and(|drag| drag.task_id == task_id)
                        && response.dragged()
                        && let Some(pointer) = response.interact_pointer_pos()
                        && let Some(drag) = self.drag
                    {
                        let delta = drag_delta_days(drag.pointer_x, pointer.x, drag.day_width);
                        if let Some(task) =
                            self.host.history_mut().current_mut().task_mut(drag.task_id)
                        {
                            task.start_day = drag_target_start(drag.original_earliest_start, delta);
                        }
                    }
                    if self.drag.is_some_and(|drag| drag.task_id == task_id)
                        && response.drag_stopped()
                    {
                        self.finish_drag();
                    }
                    if self.drag.is_some_and(|drag| drag.task_id == task_id) {
                        response.on_hover_cursor(egui::CursorIcon::Grabbing);
                    } else {
                        response
                            .on_hover_cursor(egui::CursorIcon::Grab)
                            .on_hover_text(format!(
                                "{name}\nScheduled start: {}\nDrag to reschedule.",
                                calendar.label_for_day(earliest_start)
                            ));
                    }
                }

                // A focus loss can omit the normal drag-stopped response.
                if self.drag.is_some() && ui.input(|input| !input.pointer.any_down()) {
                    self.finish_drag();
                }
            });
    }

    fn central_panel(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let mut edits = Vec::new();
        let mut messages = Vec::new();

        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Task plan");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::toolbar_button(ui, "PDF", "Export the Gantt as PDF").clicked() {
                        actions.push(Action::ExportPdf);
                    }
                    if widgets::toolbar_button(ui, "CSV", "Export the task table").clicked() {
                        actions.push(Action::ExportCsv);
                    }
                    if widgets::toolbar_button(ui, "Milestone", "Add a milestone").clicked() {
                        actions.push(Action::AddMilestone);
                    }
                    if widgets::toolbar_button(ui, "Add task", "Append a new task").clicked() {
                        actions.push(Action::AddTask);
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
                            "Duration: {} days  |  Critical: {critical}  |  Cost: {cost:.2}",
                            result.day_span()
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
                        self.details_panel(ui, &result, &mut edits, actions);
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
        if actions.iter().any(|action| {
            matches!(
                action,
                Action::New
                    | Action::Open
                    | Action::Save
                    | Action::SaveAs
                    | Action::ImportCsv
                    | Action::ExportCsv
                    | Action::ExportPdf
            )
        }) {
            self.flush_text_edits();
        }
        for action in actions.drain(..) {
            self.handle_action(action);
        }
    }

    fn handle_close_request(&mut self, ui: &egui::Ui) {
        if !ui.input(|input| input.viewport().close_requested()) {
            return;
        }
        if self.allow_close_once {
            self.allow_close_once = false;
            return;
        }
        self.finish_drag();
        self.finish_value_gesture();
        self.flush_text_edits();
        if self.pending.is_some() || self.host.is_dirty() {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if self.pending.is_none() {
                self.pending = Some(PendingAction::Exit);
            }
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
            self.resolve_pending(choice, ctx);
        } else if !open {
            self.pending = None;
        }
    }
}

impl eframe::App for ProjectApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let mut actions = Vec::new();
        if let Some(action) = shortcut_action(ui) {
            actions.push(action);
        }
        if let Some(action) = self.top_bar(ui) {
            actions.push(action);
        }
        self.status_bar(ui);
        self.central_panel(ui, &mut actions);
        self.handle_close_request(ui);
        self.unsaved_window(ui.ctx());
        self.shell.show_windows(ui.ctx());
        self.shell
            .update_title(ui.ctx(), &self.host.display_name(), self.host.is_dirty());
    }
}

fn shortcut_action(ui: &egui::Ui) -> Option<Action> {
    // Read context memory before taking egui's mutable input lock.
    let wants_keyboard_input = ui.ctx().egui_wants_keyboard_input();
    ui.input_mut(|input| {
        if input.consume_key(
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            egui::Key::S,
        ) {
            Some(Action::SaveAs)
        } else if input.consume_key(egui::Modifiers::COMMAND, egui::Key::S) {
            Some(Action::Save)
        } else if wants_keyboard_input {
            None
        } else if input.consume_key(
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            egui::Key::Z,
        ) {
            Some(Action::Redo)
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
    })
}

fn begin_value_gesture(
    host: &mut DocumentHost<Document>,
    active: &mut bool,
    response: &egui::Response,
) {
    if !*active && (response.gained_focus() || response.drag_started() || response.changed()) {
        host.history_mut().begin_gesture();
        *active = true;
    }
}

fn finish_value_gesture(host: &mut DocumentHost<Document>, active: &mut bool) {
    if *active {
        host.history_mut().end_gesture();
        *active = false;
    }
}

fn finish_value_gesture_if(
    host: &mut DocumentHost<Document>,
    active: &mut bool,
    response: &egui::Response,
) {
    if response.drag_stopped() || response.lost_focus() {
        finish_value_gesture(host, active);
    }
}

fn replace_with_import(host: &mut DocumentHost<Document>, document: Document) {
    let previous = host.document().clone();
    *host = DocumentHost::new(Document::default(), FORMAT, SCHEMA_VERSION, APP_VERSION);
    host.apply(|current| *current = previous);
    host.apply(|current| *current = document);
}

fn sort_header_label(key: SortKey, selected: bool) -> String {
    if selected {
        format!("{} v", key.label())
    } else {
        key.label().to_owned()
    }
}

fn task_id_label(id: u64, milestone: bool) -> String {
    if milestone {
        format!("M {id}")
    } else {
        id.to_string()
    }
}

fn gantt_task_label(name: &str, selected: bool) -> String {
    let mut label = String::new();
    if selected {
        label.push_str("> ");
    }
    let maximum_name_characters = 26;
    label.extend(name.chars().take(maximum_name_characters));
    if name.chars().count() > maximum_name_characters {
        label.push_str("...");
    }
    label
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
    fn shortcut_polling_does_not_deadlock_the_egui_context() {
        let context = egui::Context::default();
        let mut output = context.run_ui(egui::RawInput::default(), |context| {
            egui::CentralPanel::default().show(context, |ui| {
                assert!(shortcut_action(ui).is_none());
            });
        });
        output.textures_delta.clear();
    }

    #[test]
    fn slugify_produces_a_safe_file_name() {
        assert_eq!(slugify("Sample project!"), "sample-project");
        assert_eq!(slugify("  "), "untitled");
    }

    #[test]
    fn ui_markers_do_not_depend_on_missing_font_glyphs() {
        assert!(sort_header_label(SortKey::Name, true).is_ascii());
        assert!(task_id_label(7, true).is_ascii());
        assert!(gantt_task_label("Implementation", true).is_ascii());
        assert_eq!(gantt_task_label("Implementation", true), "> Implementation");
    }

    #[test]
    fn long_gantt_labels_are_bounded() {
        let label = gantt_task_label(&"x".repeat(100), false);
        assert!(label.ends_with("..."));
        assert!(label.chars().count() <= 29);
    }

    #[test]
    fn csv_import_is_unsaved_and_does_not_overwrite_the_open_project() {
        let path =
            std::env::temp_dir().join(format!("projectcraft-import-{}.json", std::process::id()));
        let mut host = DocumentHost::new(Document::sample(), FORMAT, SCHEMA_VERSION, APP_VERSION);
        host.save_as(&path).expect("save original");
        host.apply(|document| document.title = "Unsaved previous".into());
        let previous = host.document().clone();
        let imported = Document {
            title: "Imported project".into(),
            ..Document::default()
        };

        replace_with_import(&mut host, imported.clone());

        assert!(host.path().is_none());
        assert!(host.is_dirty());
        assert_eq!(host.document(), &imported);
        assert!(host.undo());
        assert_eq!(host.document(), &previous);
        assert!(host.is_dirty());
        assert!(host.redo());
        assert_eq!(host.document(), &imported);
        let _ = std::fs::remove_file(path);
    }
}
