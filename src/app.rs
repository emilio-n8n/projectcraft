//! ProjectCraft UI: task table, Gantt canvas and project lifecycle.

use std::collections::BTreeMap;
use std::path::PathBuf;

use craftkit_io::DocumentHost;
use craftkit_ui::cli::CliOptions;
use craftkit_ui::eframe::{self, egui};
use craftkit_ui::egui::{
    Align, Align2, CornerRadius, DragValue, FontFamily, FontId, Layout, Rect, RichText, ScrollArea,
    Sense, Stroke, TextEdit, Vec2, pos2, vec2,
};
use craftkit_ui::{AppIdentity, AppShell, Tone, dialogs, theme, widgets};

use crate::csv::export_csv as write_csv;
use crate::document::{Document, format_dependencies, parse_dependencies};
use crate::schedule::{Schedule, schedule};

const FORMAT: &str = "projectcraft";
const SCHEMA_VERSION: u32 = 1;
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Day scale of the Gantt canvas, in pixels per day.
const DAY_WIDTH: f32 = 18.0;
/// Width of the task-name gutter on the Gantt canvas.
const LABEL_WIDTH: f32 = 175.0;
/// Height of one Gantt row.
const ROW_HEIGHT: f32 = 22.0;
/// Height of the day ruler.
const HEADER_HEIGHT: f32 = 26.0;

/// Fixed cell widths of the task table. Explicit widths keep the Name column
/// readable regardless of the panel width; the table scrolls horizontally.
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
    ExportCsv,
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

/// A deferred document mutation collected while drawing the table.
#[derive(Debug, Clone)]
enum TaskEdit {
    Title(String),
    Name(u64, String),
    Dependencies(u64, Vec<u64>),
    Start(u64, i64),
    Duration(u64, i64),
    Progress(u64, f32),
}

/// The ProjectCraft application state.
pub struct ProjectApp {
    shell: AppShell,
    host: DocumentHost<Document>,
    selected: Option<u64>,
    title_buffer: String,
    name_buffers: BTreeMap<u64, String>,
    dep_buffers: BTreeMap<u64, String>,
    schedule_error: Option<String>,
    pending: Option<PendingAction>,
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
            schedule_error: None,
            pending: None,
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
        for task in &document.tasks {
            self.name_buffers.insert(task.id, task.name.clone());
            self.dep_buffers
                .insert(task.id, format_dependencies(&task.dependencies));
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

    fn delete_task(&mut self, id: u64) {
        if self.host.apply(|document| {
            document.remove_task(id);
        }) {
            if self.selected == Some(id) {
                self.selected = None;
            }
            self.name_buffers.remove(&id);
            self.dep_buffers.remove(&id);
            self.refresh_buffers();
            self.shell.set_status("Task deleted");
        }
    }

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
            Action::ExportCsv => self.export_csv(),
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
                    ("Export CSV...", Action::ExportCsv),
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

    fn task_table(
        &mut self,
        ui: &mut egui::Ui,
        edits: &mut Vec<TaskEdit>,
        messages: &mut Vec<String>,
    ) {
        let tasks = self.host.document().tasks.clone();
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
                        ui.add_sized([NAME_WIDTH, CELL_HEIGHT], egui::Label::new("Name"));
                        ui.add_sized([START_WIDTH, CELL_HEIGHT], egui::Label::new("Start"));
                        ui.add_sized([DURATION_WIDTH, CELL_HEIGHT], egui::Label::new("Days"));
                        ui.add_sized(
                            [DEPENDENCIES_WIDTH, CELL_HEIGHT],
                            egui::Label::new("Depends on"),
                        );
                        ui.add_sized([PROGRESS_WIDTH, CELL_HEIGHT], egui::Label::new("Progress"));
                        ui.add_sized([DELETE_WIDTH, CELL_HEIGHT], egui::Label::new(""));
                    });
                    ui.separator();

                    for task in &tasks {
                        ui.horizontal(|ui| {
                            let is_selected = *selected == Some(task.id);
                            let id_button =
                                egui::Button::new(task.id.to_string()).fill(if is_selected {
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

                            let delete = egui::Button::new("x");
                            if ui.add_sized([DELETE_WIDTH, CELL_HEIGHT], delete).clicked() {
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

    fn gantt(&self, ui: &mut egui::Ui, result: &Schedule) {
        let tasks = &self.host.document().tasks;
        let days = result.project_duration.max(1);
        let content_width = LABEL_WIDTH + days as f32 * DAY_WIDTH + 24.0;
        let content_height = HEADER_HEIGHT + tasks.len() as f32 * ROW_HEIGHT + 12.0;
        let font = FontId::new(11.0, FontFamily::Proportional);
        let small_font = FontId::new(9.5, FontFamily::Proportional);

        ScrollArea::both()
            .auto_shrink([false, false])
            .max_height(360.0)
            .show(ui, |ui| {
                let (rect, _) = ui
                    .allocate_exact_size(Vec2::new(content_width, content_height), Sense::hover());
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, CornerRadius::ZERO, theme::SUNKEN);

                // Alternating row backgrounds.
                for row in 0..tasks.len() {
                    if row % 2 == 0 {
                        continue;
                    }
                    let top = rect.min.y + HEADER_HEIGHT + row as f32 * ROW_HEIGHT;
                    let row_rect =
                        Rect::from_min_size(pos2(rect.min.x, top), vec2(content_width, ROW_HEIGHT));
                    painter.rect_filled(row_rect, CornerRadius::ZERO, theme::PANEL);
                }

                // Ruler background and day ticks.
                let header = Rect::from_min_size(rect.min, vec2(content_width, HEADER_HEIGHT));
                painter.rect_filled(header, CornerRadius::ZERO, theme::RAISED);
                let step = if days > 40 { 5 } else { 1 };
                for day in (0..=days).step_by(step) {
                    let x = rect.min.x + LABEL_WIDTH + day as f32 * DAY_WIDTH;
                    painter.line_segment(
                        [pos2(x, rect.min.y), pos2(x, rect.max.y)],
                        Stroke::new(1.0, theme::BORDER),
                    );
                    painter.text(
                        pos2(x + 3.0, rect.min.y + HEADER_HEIGHT / 2.0),
                        Align2::LEFT_CENTER,
                        day.to_string(),
                        small_font.clone(),
                        theme::MUTED,
                    );
                }
                painter.text(
                    pos2(rect.min.x + 8.0, rect.min.y + HEADER_HEIGHT / 2.0),
                    Align2::LEFT_CENTER,
                    "Task / day",
                    small_font,
                    theme::FAINT,
                );

                // Bars and labels.
                for (row, task) in tasks.iter().enumerate() {
                    let center_y =
                        rect.min.y + HEADER_HEIGHT + row as f32 * ROW_HEIGHT + ROW_HEIGHT / 2.0;
                    painter.text(
                        pos2(rect.min.x + 8.0, center_y),
                        Align2::LEFT_CENTER,
                        task.name.as_str(),
                        font.clone(),
                        theme::TEXT,
                    );
                    let Some(timing) = result.task(task.id) else {
                        continue;
                    };
                    let left = rect.min.x + LABEL_WIDTH + timing.earliest_start as f32 * DAY_WIDTH;
                    let right =
                        rect.min.x + LABEL_WIDTH + timing.earliest_finish as f32 * DAY_WIDTH;
                    let bar = Rect::from_min_max(
                        pos2(left, center_y - 7.0),
                        pos2(right.max(left + 2.0), center_y + 7.0),
                    );
                    let color = if timing.is_critical() {
                        theme::ACCENT
                    } else {
                        theme::INFO
                    };
                    painter.rect_filled(bar, CornerRadius::same(4), color);
                    if task.progress > 0.0 {
                        let width = bar.width() * task.progress.clamp(0.0, 1.0);
                        let done = Rect::from_min_size(bar.min, vec2(width, bar.height()));
                        painter.rect_filled(done, CornerRadius::same(4), theme::SUCCESS);
                    }
                    if !timing.is_critical() && timing.total_float > 0 {
                        let slack_end =
                            rect.min.x + LABEL_WIDTH + timing.latest_finish as f32 * DAY_WIDTH;
                        painter.line_segment(
                            [pos2(right, center_y), pos2(slack_end, center_y)],
                            Stroke::new(2.0, theme::FAINT),
                        );
                    }
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
                    if widgets::toolbar_button(ui, "Add task", "Append a new task").clicked() {
                        self.add_task();
                    }
                    if widgets::toolbar_button(ui, "Export CSV", "Export the task table").clicked()
                    {
                        self.export_csv();
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
            ui.separator();

            match schedule(self.host.document()) {
                Ok(result) => {
                    self.schedule_error = None;
                    let critical = result.critical_task_ids().len();
                    ui.label(
                        RichText::new(format!(
                            "Duration: {} days  •  Critical tasks: {critical}",
                            result.project_duration
                        ))
                        .color(theme::MUTED),
                    );
                    self.task_table(ui, &mut edits, &mut messages);
                    ui.separator();
                    self.gantt(ui, &result);
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
