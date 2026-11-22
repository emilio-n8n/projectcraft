//! CSV import and export for the task table.

use std::path::Path;

use crate::calendar::Calendar;
use crate::document::{Document, Task, parse_dependencies};
use crate::schedule::Schedule;

/// Quote a field only when CSV syntax requires it.
fn escape(field: &str) -> String {
    if field.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_owned()
    }
}

/// Build the CSV content for a scheduled document.
///
/// Each row carries both the authored fields and the computed CPM dates, so
/// the export is useful on its own without opening the project again.
#[must_use]
pub fn tasks_to_csv(document: &Document, schedule: &Schedule) -> String {
    let mut output = String::from(
        "id,name,start_day,duration_days,dependencies,progress_percent,milestone,notes,resource,cost,\
         earliest_start,earliest_finish,latest_start,latest_finish,total_float,critical\n",
    );

    for (task, timing) in document.tasks.iter().zip(schedule.tasks.iter()) {
        let dependencies = task
            .dependencies
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(";");
        let progress = (task.progress.clamp(0.0, 1.0) * 100.0).round() as i32;
        let fields = [
            task.id.to_string(),
            task.name.clone(),
            task.start_day.to_string(),
            task.duration_days.to_string(),
            dependencies,
            progress.to_string(),
            if task.milestone { "yes" } else { "no" }.to_owned(),
            task.notes.clone(),
            task.resource.clone(),
            format!("{:.2}", task.cost),
            timing.earliest_start.to_string(),
            timing.earliest_finish.to_string(),
            timing.latest_start.to_string(),
            timing.latest_finish.to_string(),
            timing.total_float.to_string(),
            if timing.is_critical() {
                "yes".to_owned()
            } else {
                "no".to_owned()
            },
        ];
        let line: Vec<String> = fields.iter().map(|field| escape(field)).collect();
        output.push_str(&line.join(","));
        output.push('\n');
    }

    output
}

/// Write the task table to `path`.
///
/// # Errors
///
/// Returns a message naming the path when the file cannot be written.
pub fn export_csv(path: &Path, document: &Document, schedule: &Schedule) -> Result<(), String> {
    std::fs::write(path, tasks_to_csv(document, schedule))
        .map_err(|error| format!("Could not write {}: {error}", path.display()))
}

/// Split CSV text into rows of unescaped fields.
///
/// Handles quoted fields, doubled quotes, commas inside quotes and both LF and
/// CRLF line endings. A leading UTF-8 BOM is ignored.
fn parse_rows(input: &str) -> Result<Vec<Vec<String>>, String> {
    let text = input.strip_prefix('\u{feff}').unwrap_or(input);
    let mut rows = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();

    while let Some(character) = chars.next() {
        match character {
            '"' if quoted => {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
            }
            '"' if field.is_empty() => quoted = true,
            ',' if !quoted => {
                row.push(std::mem::take(&mut field));
            }
            '\n' if !quoted => {
                if field.ends_with('\r') {
                    field.pop();
                }
                row.push(std::mem::take(&mut field));
                if row.iter().any(|value| !value.is_empty()) {
                    rows.push(std::mem::take(&mut row));
                } else {
                    row.clear();
                }
            }
            '\r' if !quoted && chars.peek() == Some(&'\n') => {}
            other => field.push(other),
        }
    }
    if quoted {
        return Err("Unterminated quoted CSV field".to_owned());
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        if row.iter().any(|value| !value.is_empty()) {
            rows.push(row);
        }
    }
    Ok(rows)
}

fn find_column(header: &[String], names: &[&str]) -> Option<usize> {
    header.iter().position(|value| {
        names
            .iter()
            .any(|name| value.trim().eq_ignore_ascii_case(name))
    })
}

fn field(row: &[String], column: Option<usize>) -> &str {
    column
        .and_then(|index| row.get(index))
        .map_or("", String::as_str)
}

fn parse_progress(raw: &str) -> Result<f32, String> {
    let value = raw
        .trim()
        .parse::<f32>()
        .map_err(|_| format!("Invalid progress '{raw}'"))?;
    Ok(if value > 1.0 { value / 100.0 } else { value }.clamp(0.0, 1.0))
}

fn parse_bool(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "1" | "yes" | "y" | "true"
    )
}

/// Parse a task CSV into a new document.
///
/// The exported column names are accepted, as is a minimal
/// `name,start_day,duration_days` header. Unknown columns are ignored; computed
/// CPM columns are read but not stored.
///
/// # Errors
///
/// Returns a message naming the offending row or column for malformed input.
pub fn tasks_from_csv(input: &str) -> Result<Document, String> {
    let rows = parse_rows(input)?;
    let (header, body) = rows.split_first().ok_or("CSV is empty")?;
    let name_column =
        find_column(header, &["name", "task", "task name"]).ok_or("CSV needs a 'name' column")?;
    let id_column = find_column(header, &["id"]);
    let start_column = find_column(header, &["start_day", "start", "start day"]);
    let duration_column = find_column(header, &["duration_days", "duration", "days"]);
    let dependencies_column = find_column(header, &["dependencies", "depends_on"]);
    let progress_column = find_column(header, &["progress_percent", "progress"]);
    let milestone_column = find_column(header, &["milestone"]);
    let notes_column = find_column(header, &["notes"]);
    let resource_column = find_column(header, &["resource"]);
    let cost_column = find_column(header, &["cost"]);

    let mut document = Document {
        title: "Imported project".into(),
        ..Document::default()
    };
    let mut used_ids = std::collections::HashSet::new();

    for (index, row) in body.iter().enumerate() {
        let line = index + 2;
        let name = field(row, Some(name_column)).trim().to_owned();
        if name.is_empty() {
            return Err(format!("Row {line}: empty task name"));
        }
        let start_day = parse_i64(field(row, start_column), "start_day", line)?;
        let duration_days = parse_i64(field(row, duration_column), "duration_days", line)?;
        if duration_days < 0 {
            return Err(format!("Row {line}: duration_days cannot be negative"));
        }
        let dependencies = parse_dependencies(field(row, dependencies_column))
            .map_err(|error| format!("Row {line}: {error}"))?;
        let progress = if let Some(column) = progress_column {
            parse_progress(field(row, Some(column)))
                .map_err(|error| format!("Row {line}: {error}"))?
        } else {
            0.0
        };
        let cost = if let Some(column) = cost_column {
            let raw = field(row, Some(column));
            if raw.trim().is_empty() {
                0.0
            } else {
                raw.trim()
                    .parse::<f64>()
                    .map_err(|_| format!("Row {line}: invalid cost '{raw}'"))?
            }
        } else {
            0.0
        };
        let id = match id_column.and_then(|column| row.get(column)) {
            Some(raw) if !raw.trim().is_empty() => raw
                .trim()
                .parse::<u64>()
                .map_err(|_| format!("Row {line}: invalid id '{raw}'"))?,
            _ => document.next_id,
        };
        if !used_ids.insert(id) {
            return Err(format!("Row {line}: duplicate task id {id}"));
        }
        document.next_id = document.next_id.max(id.saturating_add(1));
        document.tasks.push(Task {
            id,
            name,
            start_day,
            duration_days,
            dependencies,
            progress,
            milestone: parse_bool(field(row, milestone_column)),
            notes: field(row, notes_column).to_owned(),
            resource: field(row, resource_column).to_owned(),
            cost,
        });
    }
    document.calendar = Calendar::default();
    Ok(document)
}

fn parse_i64(raw: &str, name: &str, line: usize) -> Result<i64, String> {
    let value = raw.trim();
    if value.is_empty() {
        return Ok(0);
    }
    value
        .parse::<i64>()
        .map_err(|_| format!("Row {line}: invalid {name} '{value}'"))
}

/// Read a task CSV from `path`.
///
/// # Errors
///
/// Returns an I/O or parsing error with a descriptive message.
pub fn import_csv(path: &Path) -> Result<Document, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    let text =
        String::from_utf8(bytes).map_err(|_| format!("{} is not valid UTF-8", path.display()))?;
    tasks_from_csv(&text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schedule::schedule;

    #[test]
    fn csv_contains_header_and_computed_values() {
        let document = Document::sample();
        let result = schedule(&document).expect("sample");
        let csv = tasks_to_csv(&document, &result);
        let mut lines = csv.lines();
        assert!(
            lines
                .next()
                .expect("header")
                .starts_with("id,name,start_day,")
        );
        let rows: Vec<&str> = lines.collect();
        assert_eq!(rows.len(), 5);
        assert!(rows[0].starts_with("1,Requirements,0,2,,"));
        assert!(rows[3].ends_with(",yes"));
        assert!(rows[4].ends_with(",no"));
    }

    #[test]
    fn csv_escapes_commas_and_quotes() {
        let mut document = Document::sample();
        document.tasks[0].name = "Design, build \"and\" test".into();
        let result = schedule(&document).expect("sample");
        let csv = tasks_to_csv(&document, &result);
        assert!(csv.contains("\"Design, build \"\"and\"\" test\""));
    }

    #[test]
    fn export_then_import_round_trips_authored_fields() {
        let original = Document::sample();
        let result = schedule(&original).expect("sample");
        let imported = tasks_from_csv(&tasks_to_csv(&original, &result)).expect("import");
        assert_eq!(imported.tasks.len(), 5);
        for (before, after) in original.tasks.iter().zip(imported.tasks.iter()) {
            assert_eq!(before.id, after.id);
            assert_eq!(before.name, after.name);
            assert_eq!(before.start_day, after.start_day);
            assert_eq!(before.duration_days, after.duration_days);
            assert_eq!(before.dependencies, after.dependencies);
            assert_eq!(before.milestone, after.milestone);
            assert_eq!(before.notes, after.notes);
            assert_eq!(before.resource, after.resource);
            assert!((before.cost - after.cost).abs() < 0.01);
        }
    }

    #[test]
    fn import_accepts_a_minimal_header() {
        let csv = "name,start_day,duration_days\nAlpha,2,3\n\"Beta, B\",0,1\n";
        let document = tasks_from_csv(csv).expect("import");
        assert_eq!(document.tasks.len(), 2);
        assert_eq!(document.tasks[0].name, "Alpha");
        assert_eq!(document.tasks[0].start_day, 2);
        assert_eq!(document.tasks[1].name, "Beta, B");
        assert_eq!(document.next_id, 3);
    }

    #[test]
    fn import_reports_bad_rows() {
        assert!(tasks_from_csv("title,start\nA,0\n").is_err());
        assert!(
            tasks_from_csv("name,start_day\nA,not-a-day\n")
                .expect_err("bad number")
                .contains("Row 2")
        );
        assert!(tasks_from_csv("name,duration_days\nA,-2\n").is_err());
        assert!(tasks_from_csv("name\n\"unfinished\n").is_err());
    }

    #[test]
    fn export_writes_the_same_content() {
        let path = std::env::temp_dir().join(format!("projectcraft-csv-{}", std::process::id()));
        let document = Document::sample();
        let result = schedule(&document).expect("sample");
        export_csv(&path, &document, &result).expect("export");
        let content = std::fs::read_to_string(&path).expect("read");
        assert_eq!(content, tasks_to_csv(&document, &result));
        let _ = std::fs::remove_file(&path);
    }
}
