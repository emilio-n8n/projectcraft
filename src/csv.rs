//! CSV import and export for the task table.

use std::collections::HashSet;
use std::path::Path;

use crate::calendar::Calendar;
use crate::document::{Document, Task, parse_dependencies};
use crate::schedule::{Schedule, schedule};

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
        let progress = f64::from(task.progress.clamp(0.0, 1.0)) * 100.0;
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
            task.cost.to_string(),
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

/// Write the task table to `path` atomically.
///
/// # Errors
///
/// Returns a message naming the path when the file cannot be written.
pub fn export_csv(path: &Path, document: &Document, schedule: &Schedule) -> Result<(), String> {
    craftkit_io::atomic::write_atomic(path, tasks_to_csv(document, schedule).as_bytes())
        .map_err(|error| format!("Could not write {}: {error}", path.display()))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CsvState {
    Unquoted,
    Quoted,
    AfterQuote,
}

/// Split CSV text into rows of unescaped fields.
///
/// Handles quoted fields, doubled quotes, commas and newlines inside quotes,
/// LF/CRLF/CR record endings and a leading UTF-8 BOM. Malformed quoting is
/// rejected instead of being silently reinterpreted.
fn parse_rows(input: &str) -> Result<Vec<Vec<String>>, String> {
    let text = input.strip_prefix('\u{feff}').unwrap_or(input);
    let mut rows = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut state = CsvState::Unquoted;
    let mut touched = false;
    let mut line = 1_usize;
    let mut chars = text.chars().peekable();

    while let Some(character) = chars.next() {
        if state == CsvState::Quoted {
            match character {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => state = CsvState::AfterQuote,
                '\n' => {
                    field.push('\n');
                    line += 1;
                }
                other => field.push(other),
            }
            continue;
        }

        match character {
            ',' => {
                row.push(std::mem::take(&mut field));
                touched = true;
                state = CsvState::Unquoted;
            }
            '\n' | '\r' => {
                if character == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                finish_row(&mut rows, &mut row, &mut field, &mut touched);
                line += 1;
            }
            '"' if state == CsvState::Unquoted && field.is_empty() => {
                state = CsvState::Quoted;
                touched = true;
            }
            '"' => {
                return Err(format!(
                    "Line {line}: unexpected quote in an unquoted CSV field"
                ));
            }
            _other if state == CsvState::AfterQuote => {
                return Err(format!(
                    "Line {line}: unexpected character after a closing quote"
                ));
            }
            other => {
                field.push(other);
                touched = true;
            }
        }
    }

    if state == CsvState::Quoted {
        return Err(format!("Line {line}: unterminated quoted CSV field"));
    }
    if touched || !field.is_empty() || !row.is_empty() {
        finish_row(&mut rows, &mut row, &mut field, &mut touched);
    }
    Ok(rows)
}

fn finish_row(
    rows: &mut Vec<Vec<String>>,
    row: &mut Vec<String>,
    field: &mut String,
    touched: &mut bool,
) {
    row.push(std::mem::take(field));
    if *touched {
        rows.push(std::mem::take(row));
    } else {
        row.clear();
    }
    *touched = false;
}

fn find_column(header: &[String], names: &[&str]) -> Result<Option<usize>, String> {
    let matches: Vec<usize> = header
        .iter()
        .enumerate()
        .filter_map(|(index, value)| {
            names
                .iter()
                .any(|name| value.trim().eq_ignore_ascii_case(name))
                .then_some(index)
        })
        .collect();
    if matches.len() > 1 {
        return Err(format!("CSV has duplicate columns for '{}'", names[0]));
    }
    Ok(matches.first().copied())
}

fn field(row: &[String], column: Option<usize>) -> &str {
    column
        .and_then(|index| row.get(index))
        .map_or("", String::as_str)
}

fn parse_progress(raw: &str, percent_column: bool) -> Result<f32, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(0.0);
    }
    let value = raw
        .parse::<f64>()
        .map_err(|_| format!("Invalid progress '{raw}'"))?;
    if !value.is_finite() {
        return Err(format!("Invalid progress '{raw}'"));
    }
    let ratio = if percent_column || value > 1.0 {
        value / 100.0
    } else {
        value
    };
    if !ratio.is_finite() || !(0.0..=1.0).contains(&ratio) {
        return Err(format!("Progress '{raw}' is outside 0-100%"));
    }
    Ok(ratio as f32)
}

fn parse_bool(raw: &str) -> Result<bool, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "0" | "no" | "n" | "false" => Ok(false),
        "1" | "yes" | "y" | "true" => Ok(true),
        _ => Err(format!("Invalid milestone value '{}'", raw.trim())),
    }
}

/// Parse a task CSV into a new document.
///
/// The exported column names are accepted, as is a minimal
/// `name,start_day,duration_days` header. Unknown columns are ignored; computed
/// CPM columns are read but not stored. Every data row must have the same number
/// of fields as the header, and the imported dependency graph must be valid.
///
/// # Errors
///
/// Returns a message naming the offending row or column for malformed input.
pub fn tasks_from_csv(input: &str) -> Result<Document, String> {
    let rows = parse_rows(input)?;
    let (header, body) = rows.split_first().ok_or("CSV is empty")?;
    if header.iter().any(|value| value.trim().is_empty()) {
        return Err("CSV header contains an empty column name".to_owned());
    }
    let name_column =
        find_column(header, &["name", "task", "task name"])?.ok_or("CSV needs a 'name' column")?;
    let id_column = find_column(header, &["id"])?;
    let start_column = find_column(header, &["start_day", "start", "start day"])?;
    let duration_column = find_column(header, &["duration_days", "duration", "days"])?;
    let dependencies_column = find_column(header, &["dependencies", "depends_on", "depends on"])?;
    let progress_column = find_column(header, &["progress_percent", "progress"])?;
    let progress_is_percent = header
        .get(progress_column.unwrap_or(usize::MAX))
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("progress_percent"));
    let milestone_column = find_column(header, &["milestone"])?;
    let notes_column = find_column(header, &["notes"])?;
    let resource_column = find_column(header, &["resource"])?;
    let cost_column = find_column(header, &["cost"])?;

    let mut document = Document {
        title: "Imported project".into(),
        ..Document::default()
    };
    let mut used_ids = HashSet::new();

    for (index, row) in body.iter().enumerate() {
        let line = index + 2;
        if row.len() != header.len() {
            return Err(format!(
                "Row {line}: expected {} columns, found {}",
                header.len(),
                row.len()
            ));
        }
        let name = field(row, Some(name_column)).to_owned();
        if name.trim().is_empty() {
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
            parse_progress(field(row, Some(column)), progress_is_percent)
                .map_err(|error| format!("Row {line}: {error}"))?
        } else {
            0.0
        };
        let milestone = parse_bool(field(row, milestone_column))
            .map_err(|error| format!("Row {line}: {error}"))?;
        if milestone && duration_days != 0 {
            return Err(format!("Row {line}: a milestone must have zero duration"));
        }
        let cost = if let Some(column) = cost_column {
            let raw = field(row, Some(column));
            if raw.trim().is_empty() {
                0.0
            } else {
                let value = raw
                    .trim()
                    .parse::<f64>()
                    .map_err(|_| format!("Row {line}: invalid cost '{raw}'"))?;
                if !value.is_finite() {
                    return Err(format!("Row {line}: invalid cost '{raw}'"));
                }
                value
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
            milestone,
            notes: field(row, notes_column).to_owned(),
            resource: field(row, resource_column).to_owned(),
            cost,
        });
    }
    schedule(&document).map_err(|error| format!("Invalid plan: {error}"))?;
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
    fn csv_escapes_commas_quotes_and_newlines() {
        let mut document = Document::sample();
        document.tasks[0].name = "Design, build \"and\" test".into();
        document.tasks[0].notes = "First line\nSecond \"quoted\" line".into();
        let result = schedule(&document).expect("sample");
        let csv = tasks_to_csv(&document, &result);
        assert!(csv.contains("\"Design, build \"\"and\"\" test\""));
        assert!(csv.contains("\"First line\nSecond \"\"quoted\"\" line\""));
        let imported = tasks_from_csv(&csv).expect("round trip");
        assert_eq!(imported.tasks[0].name, document.tasks[0].name);
        assert_eq!(imported.tasks[0].notes, document.tasks[0].notes);
    }

    #[test]
    fn export_then_import_round_trips_authored_fields_exactly() {
        let mut original = Document::sample();
        original.tasks[0].name = "  Requirements, revised  ".into();
        original.tasks[0].progress = 0.333_333_34;
        original.tasks[0].cost = 1_234.567_89;
        original.tasks[1].progress = 0.01;
        let result = schedule(&original).expect("sample");
        let imported = tasks_from_csv(&tasks_to_csv(&original, &result)).expect("import");
        assert_eq!(imported.tasks.len(), 5);
        for (before, after) in original.tasks.iter().zip(imported.tasks.iter()) {
            assert_eq!(before.id, after.id);
            assert_eq!(before.name, after.name);
            assert_eq!(before.start_day, after.start_day);
            assert_eq!(before.duration_days, after.duration_days);
            assert_eq!(before.dependencies, after.dependencies);
            assert_eq!(before.progress.to_bits(), after.progress.to_bits());
            assert_eq!(before.milestone, after.milestone);
            assert_eq!(before.notes, after.notes);
            assert_eq!(before.resource, after.resource);
            assert_eq!(before.cost.to_bits(), after.cost.to_bits());
        }
    }

    #[test]
    fn progress_percent_one_means_one_percent_not_complete() {
        let document =
            tasks_from_csv("name,progress_percent\nLow,1\nComplete,100\n").expect("import");
        assert_eq!(document.tasks[0].progress.to_bits(), 0.01_f32.to_bits());
        assert_eq!(document.tasks[1].progress, 1.0);
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
    fn import_accepts_bom_crlf_and_quoted_crlf() {
        let csv =
            "\u{feff}name,start_day,duration_days,notes\r\nAlpha,0,1,\"line 1\r\nline 2\"\r\n\r\n";
        let document = tasks_from_csv(csv).expect("import");
        assert_eq!(document.tasks.len(), 1);
        assert_eq!(document.tasks[0].notes, "line 1\r\nline 2");
    }

    #[test]
    fn import_reports_bad_rows_and_malformed_quoting() {
        assert!(tasks_from_csv("title,start\nA,0\n").is_err());
        assert!(
            tasks_from_csv("name,start_day\nA,not-a-day\n")
                .expect_err("bad number")
                .contains("Row 2")
        );
        assert!(tasks_from_csv("name,duration_days\nA,-2\n").is_err());
        assert!(tasks_from_csv("name\n\"unfinished\n").is_err());
        assert!(tasks_from_csv("name\nA\"bad\n").is_err());
        assert!(tasks_from_csv("name\n\"A\"trailing\n").is_err());
        assert!(
            tasks_from_csv("name,cost\nA,1,extra\n")
                .expect_err("column count")
                .contains("expected 2 columns")
        );
        assert!(tasks_from_csv("name,name\nA,B\n").is_err());
    }

    #[test]
    fn import_rejects_non_finite_and_invalid_typed_values() {
        assert!(tasks_from_csv("name,progress_percent\nA,NaN\n").is_err());
        assert!(tasks_from_csv("name,progress_percent\nA,101\n").is_err());
        assert!(tasks_from_csv("name,cost\nA,inf\n").is_err());
        assert!(tasks_from_csv("name,milestone\nA,maybe\n").is_err());
        assert!(tasks_from_csv("name,duration_days,milestone\nA,2,yes\n").is_err());
    }

    #[test]
    fn import_rejects_invalid_dependency_graphs() {
        assert!(
            tasks_from_csv("name,dependencies\nA,99\n")
                .expect_err("missing")
                .contains("unknown task 99")
        );
        assert!(
            tasks_from_csv("name,dependencies\nA,1\n")
                .expect_err("self")
                .contains("itself")
        );
        assert!(
            tasks_from_csv("name,dependencies\nA,2\nB,1\n")
                .expect_err("cycle")
                .contains("cycle")
        );
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
