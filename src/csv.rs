//! CSV bill-of-tasks export.

use std::path::Path;

use crate::document::Document;
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
        "id,name,start_day,duration_days,dependencies,progress_percent,\
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
        assert_eq!(
            lines.next().expect("header"),
            "id,name,start_day,duration_days,dependencies,progress_percent,\
             earliest_start,earliest_finish,latest_start,latest_finish,total_float,critical"
        );
        let rows: Vec<&str> = lines.collect();
        assert_eq!(rows.len(), 5);
        assert!(rows[0].starts_with("1,Requirements,0,2,,"));
        assert!(rows[3].ends_with(",0,yes"));
        assert!(rows[4].ends_with(",3,no"));
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
    fn export_writes_the_same_content() {
        let directory =
            std::env::temp_dir().join(format!("projectcraft-csv-{}", std::process::id()));
        let path = directory.with_extension("csv");
        let document = Document::sample();
        let result = schedule(&document).expect("sample");
        export_csv(&path, &document, &result).expect("export");
        let content = std::fs::read_to_string(&path).expect("read");
        assert_eq!(content, tasks_to_csv(&document, &result));
        let _ = std::fs::remove_file(&path);
    }
}
