//! Project document model.
//!
//! A document is a small, self-contained project plan. Every scheduling value
//! is stored in plain integer days. Newer, optional presentation fields use
//! `#[serde(default)]` so projects written by v0.1.0 still load under schema 1.

use serde::{Deserialize, Serialize};

use crate::calendar::Calendar;

/// A single planned task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    /// Stable identifier, unique inside a document.
    pub id: u64,
    /// Human-readable task name.
    pub name: String,
    /// Earliest day on which the task may start (release constraint).
    pub start_day: i64,
    /// Length of the task in whole days.
    pub duration_days: i64,
    /// Identifiers of tasks that must finish before this one starts.
    pub dependencies: Vec<u64>,
    /// Completion ratio, from `0.0` (not started) to `1.0` (done).
    pub progress: f32,
    /// A milestone is a zero-duration marker drawn as a diamond.
    #[serde(default)]
    pub milestone: bool,
    /// Free-form task notes.
    #[serde(default)]
    pub notes: String,
    /// Assigned resource or team.
    #[serde(default)]
    pub resource: String,
    /// Estimated cost in project currency units.
    #[serde(default)]
    pub cost: f64,
}

/// A versioned project plan owned by the application.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    /// Project title.
    pub title: String,
    /// Tasks in display order.
    pub tasks: Vec<Task>,
    /// Next identifier handed out by [`Document::add_task`].
    pub next_id: u64,
    /// Optional real-date settings for the Gantt ruler.
    #[serde(default)]
    pub calendar: Calendar,
}

impl Default for Document {
    fn default() -> Self {
        Self {
            title: "Untitled".into(),
            tasks: Vec::new(),
            next_id: 1,
            calendar: Calendar::default(),
        }
    }
}

impl Document {
    /// Built-in demonstration project with one parallel, non-critical branch.
    #[must_use]
    pub fn sample() -> Self {
        Self {
            title: "Sample project".into(),
            next_id: 6,
            tasks: vec![
                Task {
                    id: 1,
                    name: "Requirements".into(),
                    start_day: 0,
                    duration_days: 2,
                    dependencies: Vec::new(),
                    progress: 1.0,
                    milestone: false,
                    notes: "Interviews with the operations team.".into(),
                    resource: "Product owner".into(),
                    cost: 1_200.0,
                },
                Task {
                    id: 2,
                    name: "Design".into(),
                    start_day: 0,
                    duration_days: 3,
                    dependencies: vec![1],
                    progress: 0.8,
                    milestone: false,
                    notes: String::new(),
                    resource: "Designer".into(),
                    cost: 2_400.0,
                },
                Task {
                    id: 3,
                    name: "Implementation".into(),
                    start_day: 0,
                    duration_days: 5,
                    dependencies: vec![2],
                    progress: 0.3,
                    milestone: false,
                    notes: "Server work and data migration.".into(),
                    resource: "Engineering".into(),
                    cost: 8_000.0,
                },
                Task {
                    id: 4,
                    name: "Testing".into(),
                    start_day: 0,
                    duration_days: 2,
                    dependencies: vec![3],
                    progress: 0.0,
                    milestone: false,
                    notes: String::new(),
                    resource: "QA".into(),
                    cost: 1_500.0,
                },
                Task {
                    id: 5,
                    name: "Documentation".into(),
                    start_day: 0,
                    duration_days: 4,
                    dependencies: vec![2],
                    progress: 0.1,
                    milestone: false,
                    notes: String::new(),
                    resource: "Technical writer".into(),
                    cost: 900.0,
                },
            ],
            calendar: Calendar::default(),
        }
    }

    /// Append a new empty task and return its identifier.
    pub fn add_task(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.tasks.push(Task {
            id,
            name: format!("Task {id}"),
            start_day: 0,
            duration_days: 1,
            dependencies: Vec::new(),
            progress: 0.0,
            milestone: false,
            notes: String::new(),
            resource: String::new(),
            cost: 0.0,
        });
        id
    }

    /// Append a zero-duration milestone that depends on `after`, if given.
    pub fn add_milestone(&mut self, after: Option<u64>) -> u64 {
        let id = self.add_task();
        if let Some(task) = self.task_mut(id) {
            task.name = format!("Milestone {id}");
            task.duration_days = 0;
            task.milestone = true;
            task.dependencies = after.into_iter().collect();
        }
        id
    }

    /// Copy a task with a fresh identifier and return the new identifier.
    pub fn duplicate_task(&mut self, id: u64) -> Option<u64> {
        let mut copy = self.task(id)?.clone();
        let new_id = self.add_task();
        copy.id = new_id;
        copy.name = format!("{} (copy)", copy.name);
        if let Some(original) = self.task_mut(new_id) {
            *original = copy;
        }
        Some(new_id)
    }

    /// Move a task up (`-1`) or down (`+1`) in display order.
    pub fn move_task(&mut self, id: u64, delta: isize) -> bool {
        let Some(index) = self.tasks.iter().position(|task| task.id == id) else {
            return false;
        };
        let target = index as isize + delta;
        if target < 0 || target >= self.tasks.len() as isize {
            return false;
        }
        let task = self.tasks.remove(index);
        self.tasks.insert(target as usize, task);
        true
    }

    /// Remove a task and drop its identifier from every dependency list.
    ///
    /// Keeping dangling dependencies would make the project unschedulable, so
    /// deletion always repairs the remaining graph.
    pub fn remove_task(&mut self, id: u64) -> bool {
        let before = self.tasks.len();
        self.tasks.retain(|task| task.id != id);
        let removed = self.tasks.len() != before;
        if removed {
            for task in &mut self.tasks {
                task.dependencies.retain(|dependency| *dependency != id);
            }
        }
        removed
    }

    /// Find a task by identifier.
    #[must_use]
    pub fn task(&self, id: u64) -> Option<&Task> {
        self.tasks.iter().find(|task| task.id == id)
    }

    /// Find a task by identifier for editing.
    pub fn task_mut(&mut self, id: u64) -> Option<&mut Task> {
        self.tasks.iter_mut().find(|task| task.id == id)
    }

    /// Sum of the estimated task costs.
    #[must_use]
    pub fn total_cost(&self) -> f64 {
        self.tasks.iter().map(|task| task.cost).sum()
    }
}

/// Parse the UI dependency field (`"1, 2;5"`) into unique task identifiers.
///
/// # Errors
///
/// Returns a message naming the first token that is not a positive integer.
pub fn parse_dependencies(input: &str) -> Result<Vec<u64>, String> {
    let mut ids: Vec<u64> = Vec::new();
    for token in input.split([',', ';', ' ', '\t']) {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        let id = token
            .parse::<u64>()
            .map_err(|_| format!("Invalid dependency '{token}'"))?;
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    Ok(ids)
}

/// Render dependency identifiers for the UI field.
#[must_use]
pub fn format_dependencies(ids: &[u64]) -> String {
    ids.iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_has_five_tasks_and_a_unique_id_sequence() {
        let document = Document::sample();
        assert_eq!(document.tasks.len(), 5);
        assert_eq!(document.next_id, 6);
        let mut ids: Vec<u64> = document.tasks.iter().map(|task| task.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 5);
    }

    #[test]
    fn add_and_remove_task_keep_dependencies_consistent() {
        let mut document = Document::default();
        let first = document.add_task();
        let second = document.add_task();
        assert_eq!((first, second), (1, 2));
        document.task_mut(second).expect("task").dependencies = vec![first];

        assert!(document.remove_task(first));
        assert!(!document.remove_task(first));
        assert_eq!(document.tasks.len(), 1);
        assert!(document.tasks[0].dependencies.is_empty());
    }

    #[test]
    fn legacy_schema_one_json_loads_with_defaults() {
        let legacy = r#"{
            "title": "Old project",
            "tasks": [{
                "id": 1, "name": "Old task", "start_day": 0,
                "duration_days": 2, "dependencies": [], "progress": 0.5
            }],
            "next_id": 2
        }"#;
        let document: Document = serde_json::from_str(legacy).expect("legacy project");
        assert_eq!(document.title, "Old project");
        assert_eq!(document.calendar, Calendar::default());
        let task = &document.tasks[0];
        assert!(!task.milestone);
        assert!(task.notes.is_empty());
        assert!(task.resource.is_empty());
        assert_eq!(task.cost, 0.0);
    }

    #[test]
    fn milestone_duplicate_and_reorder() {
        let mut document = Document::sample();
        let milestone = document.add_milestone(Some(4));
        let task = document.task(milestone).expect("milestone");
        assert!(task.milestone);
        assert_eq!(task.duration_days, 0);
        assert_eq!(task.dependencies, vec![4]);

        let copy = document.duplicate_task(1).expect("copy");
        assert_eq!(
            document.task(copy).expect("copy task").name,
            "Requirements (copy)"
        );
        assert!(document.move_task(copy, -1));
        assert_eq!(
            document.tasks.iter().position(|task| task.id == copy),
            Some(5)
        );
        assert!(!document.move_task(999, -1));
    }

    #[test]
    fn dependency_field_round_trip() {
        assert_eq!(
            parse_dependencies("1, 3; 2  3").expect("parse"),
            vec![1, 3, 2]
        );
        assert_eq!(format_dependencies(&[1, 3, 2]), "1, 3, 2");
        assert!(parse_dependencies("1, nope").is_err());
    }

    #[test]
    fn json_round_trip_preserves_the_sample() {
        let original = Document::sample();
        let json = serde_json::to_string(&original).expect("serialize");
        let restored: Document = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored, original);
    }

    #[test]
    fn undo_redo_of_a_task_edit() {
        let mut host =
            craftkit_io::DocumentHost::new(Document::sample(), "projectcraft", 1, "0.1.0");
        let original = host.document().tasks[1].name.clone();

        assert!(host.apply(|document| {
            if let Some(task) = document.task_mut(2) {
                task.name = "Detailed design".into();
            }
        }));
        assert!(host.is_dirty());
        assert_eq!(host.document().tasks[1].name, "Detailed design");

        assert!(host.undo());
        assert_eq!(host.document().tasks[1].name, original);
        assert!(host.redo());
        assert_eq!(host.document().tasks[1].name, "Detailed design");
    }

    #[test]
    fn drag_gesture_is_one_undo_step() {
        let mut host =
            craftkit_io::DocumentHost::new(Document::sample(), "projectcraft", 1, "0.1.0");
        let original = host.document().task(3).expect("task").start_day;

        host.history_mut().begin_gesture();
        for delta in [1_i64, 2, 3] {
            if let Some(task) = host.history_mut().current_mut().task_mut(3) {
                task.start_day = original + delta;
            }
        }
        host.history_mut().end_gesture();

        assert_eq!(host.history().undo_depth(), 1);
        assert_eq!(
            host.document().task(3).expect("task").start_day,
            original + 3
        );
        assert!(host.undo());
        assert_eq!(host.document().task(3).expect("task").start_day, original);
    }
}
