//! Critical Path Method (CPM) scheduling in whole days.
//!
//! The forward pass honours each task's release day and its predecessors; the
//! backward pass derives latest dates from the project finish. No wall-clock
//! date is ever involved, so results are deterministic and easy to test.

use std::collections::{HashMap, HashSet, VecDeque};

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
    /// Earliest finish of the whole project, in days.
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
}

/// Compute the CPM schedule of a document.
///
/// # Errors
///
/// Returns a descriptive message when task identifiers are duplicated, a
/// dependency points to a missing task, a duration is negative or the
/// dependency graph contains a cycle. A cycle error names the tasks involved.
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
        earliest_finish[index] = start + task.duration_days;
    }
    let project_duration = earliest_finish.iter().copied().max().unwrap_or(0);

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
        .map(|index| TaskSchedule {
            task_id: tasks[index].id,
            earliest_start: earliest_start[index],
            earliest_finish: earliest_finish[index],
            latest_start: latest_start[index],
            latest_finish: latest_finish[index],
            total_float: latest_start[index] - earliest_start[index],
        })
        .collect();

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
    let mut queue: VecDeque<usize> = (0..count).filter(|&index| remaining[index] == 0).collect();
    let mut order = Vec::with_capacity(count);

    while let Some(index) = queue.pop_front() {
        order.push(index);
        for &child in &successors[index] {
            remaining[child] -= 1;
            if remaining[child] == 0 {
                queue.push_back(child);
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
    fn visit(
        node: usize,
        color: &mut [u8],
        stack: &mut Vec<usize>,
        successors: &[Vec<usize>],
        blocked: &[bool],
    ) -> Option<Vec<usize>> {
        color[node] = 1;
        stack.push(node);
        for &child in &successors[node] {
            if !blocked[child] {
                continue;
            }
            match color[child] {
                1 => {
                    let start = stack.iter().position(|&item| item == child).unwrap_or(0);
                    let mut cycle: Vec<usize> = stack[start..].to_vec();
                    cycle.push(child);
                    return Some(cycle);
                }
                0 => {
                    if let Some(found) = visit(child, color, stack, successors, blocked) {
                        return Some(found);
                    }
                }
                _ => {}
            }
        }
        stack.pop();
        color[node] = 2;
        None
    }

    let mut color = vec![0_u8; blocked.len()];
    let mut stack = Vec::new();
    for index in 0..blocked.len() {
        if blocked[index]
            && color[index] == 0
            && let Some(cycle) = visit(index, &mut color, &mut stack, successors, blocked)
        {
            return cycle;
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
        };
        assert!(
            schedule(&negative)
                .expect_err("negative")
                .contains("negative duration")
        );
    }

    #[test]
    fn release_day_delays_the_forward_pass() {
        let document = Document {
            title: "Release".into(),
            next_id: 3,
            tasks: vec![task(1, "Early", 0, 2, &[]), task(2, "Late", 7, 3, &[1])],
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
}
