//! Critical Path Method (CPM) scheduling in whole days.
//!
//! The forward pass honours each task's release day and its predecessors; the
//! backward pass derives latest dates from the project finish. No wall-clock
//! date is ever involved, so results are deterministic and easy to test.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};

use crate::document::{Document, Task};

/// Timing result for one task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskSchedule {
    /// Identifier of the scheduled task.
    pub task_id: u64,
    /// Earliest day the task can start.
    pub earliest_start: i64,
    /// Earliest day the task can finish.
    pub earliest_finish: i64,
    /// Latest day the task can start without delaying the project.
    pub latest_start: i64,
    /// Latest day the task can finish without delaying the project.
    pub latest_finish: i64,
    /// Total float (slack) in days.
    pub total_float: i64,
}

impl TaskSchedule {
    /// A task is critical when it has no float.
    #[must_use]
    pub fn is_critical(&self) -> bool {
        self.total_float == 0
    }
}

/// Result of a successful scheduling pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schedule {
    /// One entry per document task, in document order.
    pub tasks: Vec<TaskSchedule>,
    /// Earliest finish day offset of the whole project (which may be negative).
    pub project_duration: i64,
}

impl Schedule {
    /// Identifiers of the tasks on the critical path, in document order.
    #[must_use]
    pub fn critical_task_ids(&self) -> Vec<u64> {
        self.tasks
            .iter()
            .filter(|task| task.is_critical())
            .map(|task| task.task_id)
            .collect()
    }

    /// Timing of one task.
    #[must_use]
    pub fn task(&self, id: u64) -> Option<&TaskSchedule> {
        self.tasks.iter().find(|task| task.task_id == id)
    }

    /// First scheduled day, which can be negative.
    #[must_use]
    pub fn first_day(&self) -> i64 {
        self.tasks
            .iter()
            .map(|task| task.earliest_start)
            .min()
            .unwrap_or(0)
    }

    /// Number of day columns needed to draw the whole schedule.
    #[must_use]
    pub fn day_span(&self) -> i64 {
        self.project_duration
            .saturating_sub(self.first_day())
            .max(0)
    }

    /// Offset of a day from the first drawn column.
    #[must_use]
    pub fn day_offset(&self, day: i64) -> i64 {
        day.saturating_sub(self.first_day())
    }
}

/// Compute the CPM schedule of a document.
///
/// # Errors
///
/// Returns a descriptive message when task identifiers are duplicated, a
/// dependency points to a missing task, a duration is negative, a milestone is
/// non-zero, dates overflow or the dependency graph contains a cycle. A cycle
/// error names the tasks involved.
pub fn schedule(document: &Document) -> Result<Schedule, String> {
    let tasks = &document.tasks;
    let count = tasks.len();
    if count == 0 {
        return Ok(Schedule {
            tasks: Vec::new(),
            project_duration: 0,
        });
    }

    let mut index_by_id: HashMap<u64, usize> = HashMap::with_capacity(count);
    for (index, task) in tasks.iter().enumerate() {
        if index_by_id.insert(task.id, index).is_some() {
            return Err(format!("Duplicate task id {}", task.id));
        }
        if task.duration_days < 0 {
            return Err(format!("Task '{}' has a negative duration", task.name));
        }
        if task.milestone && task.duration_days != 0 {
            return Err(format!("Milestone '{}' must have zero duration", task.name));
        }
    }

    // Unique predecessor indices per task, in declaration order.
    let mut predecessors: Vec<Vec<usize>> = vec![Vec::new(); count];
    let mut successors: Vec<Vec<usize>> = vec![Vec::new(); count];
    let mut indegree = vec![0usize; count];

    for (index, task) in tasks.iter().enumerate() {
        let mut seen = HashSet::new();
        for &dependency in &task.dependencies {
            if !seen.insert(dependency) {
                continue;
            }
            let Some(&parent) = index_by_id.get(&dependency) else {
                return Err(format!(
                    "Task '{}' depends on unknown task {}",
                    task.name, dependency
                ));
            };
            if parent == index {
                return Err(format!("Task '{}' depends on itself", task.name));
            }
            predecessors[index].push(parent);
            successors[parent].push(index);
            indegree[index] += 1;
        }
    }
    for list in &mut successors {
        list.sort_unstable();
    }

    let order = topological_order(tasks, &successors, &indegree)?;

    // Forward pass: earliest dates.
    let mut earliest_start = vec![0_i64; count];
    let mut earliest_finish = vec![0_i64; count];
    for &index in &order {
        let task = &tasks[index];
        let mut start = task.start_day;
        for &parent in &predecessors[index] {
            start = start.max(earliest_finish[parent]);
        }
        earliest_start[index] = start;
        earliest_finish[index] = start.checked_add(task.duration_days).ok_or_else(|| {
            format!(
                "Task '{}' has a finish day outside the supported range",
                task.name
            )
        })?;
    }
    let project_duration = earliest_finish.iter().copied().max().unwrap_or(0);
    let first_day = earliest_start.iter().copied().min().unwrap_or(0);
    project_duration
        .checked_sub(first_day)
        .ok_or_else(|| "Project date range is too large to schedule safely".to_owned())?;

    // Backward pass: latest dates, starting from the project finish.
    let mut latest_start = vec![0_i64; count];
    let mut latest_finish = vec![0_i64; count];
    for &index in order.iter().rev() {
        let mut finish = project_duration;
        for &child in &successors[index] {
            finish = finish.min(latest_start[child]);
        }
        latest_finish[index] = finish;
        latest_start[index] = finish - tasks[index].duration_days;
    }

    let scheduled = (0..count)
        .map(|index| {
            let total_float = latest_start[index]
                .checked_sub(earliest_start[index])
                .ok_or_else(|| {
                    format!(
                        "Task '{}' has total float outside the supported range",
                        tasks[index].name
                    )
                })?;
            Ok(TaskSchedule {
                task_id: tasks[index].id,
                earliest_start: earliest_start[index],
                earliest_finish: earliest_finish[index],
                latest_start: latest_start[index],
                latest_finish: latest_finish[index],
                total_float,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    Ok(Schedule {
        tasks: scheduled,
        project_duration,
    })
}

/// Kahn topological sort with a deterministic, document-order tie break.
fn topological_order(
    tasks: &[Task],
    successors: &[Vec<usize>],
    indegree: &[usize],
) -> Result<Vec<usize>, String> {
    let count = tasks.len();
    let mut remaining = indegree.to_vec();
    let mut queue: BinaryHeap<Reverse<usize>> = (0..count)
        .filter(|&index| remaining[index] == 0)
        .map(Reverse)
        .collect();
    let mut order = Vec::with_capacity(count);

    while let Some(Reverse(index)) = queue.pop() {
        order.push(index);
        for &child in &successors[index] {
            remaining[child] -= 1;
            if remaining[child] == 0 {
                queue.push(Reverse(child));
            }
        }
    }

    if order.len() == count {
        return Ok(order);
    }

    let mut blocked = vec![false; count];
    for index in 0..count {
        blocked[index] = remaining[index] > 0;
    }
    let cycle = find_cycle(successors, &blocked);
    let names: Vec<&str> = cycle
        .iter()
        .map(|&index| tasks[index].name.as_str())
        .collect();
    Err(format!("Dependency cycle detected: {}", names.join(" -> ")))
}

/// Find a cycle among blocked nodes for a readable error message.
fn find_cycle(successors: &[Vec<usize>], blocked: &[bool]) -> Vec<usize> {
    let mut color = vec![0_u8; blocked.len()];
    let mut path = Vec::new();
    let mut stack: Vec<(usize, usize)> = Vec::new();

    for start in 0..blocked.len() {
        if !blocked[start] || color[start] != 0 {
            continue;
        }
        color[start] = 1;
        path.push(start);
        stack.push((start, 0));

        while let Some(&(node, next_child)) = stack.last() {
            if next_child >= successors[node].len() {
                color[node] = 2;
                stack.pop();
                path.pop();
                continue;
            }
            if let Some((_, next)) = stack.last_mut() {
                *next += 1;
            }
            let child = successors[node][next_child];
            if !blocked[child] {
                continue;
            }
            match color[child] {
                1 => {
                    let cycle_start = path.iter().position(|&item| item == child).unwrap_or(0);
                    let mut cycle = path[cycle_start..].to_vec();
                    cycle.push(child);
                    return cycle;
                }
                0 => {
                    color[child] = 1;
                    path.push(child);
                    stack.push((child, 0));
                }
                _ => {}
            }
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Task;

    fn task(id: u64, name: &str, start: i64, duration: i64, dependencies: &[u64]) -> Task {
        Task {
            id,
            name: name.into(),
            start_day: start,
            duration_days: duration,
            dependencies: dependencies.to_vec(),
            progress: 0.0,
            milestone: false,
            notes: String::new(),
            resource: String::new(),
            cost: 0.0,
        }
    }

    #[test]
    fn cpm_on_a_known_four_task_graph() {
        let document = Document {
            title: "CPM".into(),
            next_id: 5,
            tasks: vec![
                task(1, "A", 0, 3, &[]),
                task(2, "B", 0, 4, &[1]),
                task(3, "C", 0, 2, &[1]),
                task(4, "D", 0, 3, &[2, 3]),
            ],
            ..Document::default()
        };

        let result = schedule(&document).expect("valid plan");
        assert_eq!(result.project_duration, 10);

        let a = result.task(1).expect("A");
        assert_eq!((a.earliest_start, a.earliest_finish), (0, 3));
        assert_eq!((a.latest_start, a.latest_finish), (0, 3));
        assert_eq!(a.total_float, 0);
        assert!(a.is_critical());

        let b = result.task(2).expect("B");
        assert_eq!((b.earliest_start, b.earliest_finish), (3, 7));
        assert_eq!(b.total_float, 0);

        let c = result.task(3).expect("C");
        assert_eq!((c.earliest_start, c.earliest_finish), (3, 5));
        assert_eq!((c.latest_start, c.latest_finish), (5, 7));
        assert_eq!(c.total_float, 2);
        assert!(!c.is_critical());

        let d = result.task(4).expect("D");
        assert_eq!((d.earliest_start, d.earliest_finish), (7, 10));
        assert_eq!(d.total_float, 0);

        assert_eq!(result.critical_task_ids(), vec![1, 2, 4]);
    }

    #[test]
    fn slack_is_reported_for_the_sample_parallel_branch() {
        let result = schedule(&Document::sample()).expect("sample");
        assert_eq!(result.project_duration, 12);
        assert_eq!(result.critical_task_ids(), vec![1, 2, 3, 4]);
        let docs = result.task(5).expect("documentation");
        assert_eq!((docs.earliest_start, docs.earliest_finish), (5, 9));
        assert_eq!((docs.latest_start, docs.latest_finish), (8, 12));
        assert_eq!(docs.total_float, 3);
    }

    #[test]
    fn cycle_is_detected_with_a_clear_message() {
        let document = Document {
            title: "Cycle".into(),
            next_id: 3,
            tasks: vec![task(1, "Design", 0, 1, &[2]), task(2, "Build", 0, 1, &[1])],
            ..Document::default()
        };
        let error = schedule(&document).expect_err("cycle");
        assert!(error.contains("cycle"), "{error}");
        assert!(error.contains("Design"), "{error}");
        assert!(error.contains("Build"), "{error}");
    }

    #[test]
    fn unknown_dependency_and_negative_duration_are_rejected() {
        let missing = Document {
            title: "Missing".into(),
            next_id: 2,
            tasks: vec![task(1, "A", 0, 1, &[99])],
            ..Document::default()
        };
        assert!(
            schedule(&missing)
                .expect_err("missing")
                .contains("unknown task 99")
        );

        let negative = Document {
            title: "Negative".into(),
            next_id: 2,
            tasks: vec![task(1, "A", 0, -1, &[])],
            ..Document::default()
        };
        assert!(
            schedule(&negative)
                .expect_err("negative")
                .contains("negative duration")
        );
    }

    #[test]
    fn milestones_must_have_zero_duration() {
        let mut invalid = task(1, "Release", 0, 2, &[]);
        invalid.milestone = true;
        let document = Document {
            title: "Milestone".into(),
            next_id: 2,
            tasks: vec![invalid],
            ..Document::default()
        };
        assert!(schedule(&document).expect_err("milestone").contains("zero"));
    }

    #[test]
    fn release_day_delays_the_forward_pass() {
        let document = Document {
            title: "Release".into(),
            next_id: 3,
            tasks: vec![task(1, "Early", 0, 2, &[]), task(2, "Late", 7, 3, &[1])],
            ..Document::default()
        };
        let result = schedule(&document).expect("valid");
        let late = result.task(2).expect("late");
        assert_eq!((late.earliest_start, late.earliest_finish), (7, 10));
        assert_eq!(result.project_duration, 10);
    }

    #[test]
    fn an_empty_document_has_a_zero_length_schedule() {
        let result = schedule(&Document::default()).expect("empty");
        assert!(result.tasks.is_empty());
        assert_eq!(result.project_duration, 0);
    }

    #[test]
    fn signed_days_have_a_drawable_nonnegative_span() {
        let document = Document {
            title: "Signed".into(),
            next_id: 3,
            tasks: vec![task(1, "Before", -5, 2, &[]), task(2, "After", -3, 1, &[1])],
            ..Document::default()
        };
        let result = schedule(&document).expect("signed plan");
        assert_eq!(result.project_duration, -2);
        assert_eq!(result.first_day(), -5);
        assert_eq!(result.day_span(), 3);
        assert_eq!(result.day_offset(-5), 0);
        assert_eq!(result.day_offset(-2), 3);
    }

    #[test]
    fn date_arithmetic_overflow_is_reported_instead_of_wrapping() {
        let overflow = Document {
            title: "Overflow".into(),
            next_id: 2,
            tasks: vec![task(1, "Huge", i64::MAX, 1, &[])],
            ..Document::default()
        };
        assert!(schedule(&overflow).expect_err("overflow").contains("range"));

        let wide = Document {
            title: "Wide".into(),
            next_id: 3,
            tasks: vec![
                task(1, "Early", i64::MIN, 0, &[]),
                task(2, "Late", i64::MAX, 0, &[]),
            ],
            ..Document::default()
        };
        assert!(schedule(&wide).expect_err("wide").contains("too large"));
    }

    #[test]
    fn topological_order_uses_document_order_for_every_ready_task() {
        let tasks = vec![
            task(1, "A", 0, 1, &[]),
            task(2, "B", 0, 1, &[1]),
            task(3, "C", 0, 1, &[]),
        ];
        let order =
            topological_order(&tasks, &[vec![1], vec![], vec![]], &[0, 1, 0]).expect("order");
        assert_eq!(order, vec![0, 1, 2]);
    }

    #[test]
    fn a_deep_cycle_is_reported_without_recursive_traversal() {
        let count = 10_000_u64;
        let tasks: Vec<Task> = (0..count)
            .map(|offset| {
                let id = offset + 1;
                let dependency = if id == count { 1 } else { id + 1 };
                task(id, "T", 0, 1, &[dependency])
            })
            .collect();
        let document = Document {
            title: "Deep cycle".into(),
            next_id: count + 1,
            tasks,
            ..Document::default()
        };
        assert!(schedule(&document).expect_err("cycle").contains("cycle"));
    }
}
