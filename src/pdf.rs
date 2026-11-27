//! Minimal PDF export of the Gantt chart.
//!
//! Writes a valid PDF 1.4 document with vector bars, dependency arrows and text.
//! No third-party PDF crate or external tool is required.

use std::fmt::Write as _;

use crate::document::Document;
use crate::gantt::{LABEL_WIDTH, arrow_points, bar_rect, day_x};
use crate::schedule::Schedule;

const PAGE_WIDTH: f32 = 842.0; // A4 landscape
const PAGE_HEIGHT: f32 = 595.0;
const LEFT: f32 = 48.0;
const TOP: f32 = 540.0;
const ROW_STEP: f32 = 20.0;
const ROWS_PER_PAGE: usize = 24;
const MAX_RULER_TICKS: i64 = 80;
const CHART_START: f32 = LEFT + LABEL_WIDTH * 0.8;

/// Escape a string for a PDF literal using WinAnsi character codes.
fn escape(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '(' => output.push_str("\\("),
            ')' => output.push_str("\\)"),
            '\n' | '\r' | '\t' => output.push(' '),
            ' '..='~' => output.push(character),
            '\u{20ac}' => output.push_str("\\200"),
            '\u{00a0}'..='\u{00ff}' => {
                let _ = write!(output, "\\{:03o}", character as u32);
            }
            _ => output.push('?'),
        }
    }
    output
}

fn fit_text(text: &str, maximum_characters: usize) -> String {
    if text.chars().count() <= maximum_characters {
        return text.to_owned();
    }
    let retained = maximum_characters.saturating_sub(3);
    let mut fitted: String = text.chars().take(retained).collect();
    fitted.push_str("...");
    fitted
}

fn color(red: f32, green: f32, blue: f32) -> String {
    format!("{red:.3} {green:.3} {blue:.3} rg\n")
}

fn fill_rect(x: f32, y: f32, width: f32, height: f32) -> String {
    format!("{x:.2} {y:.2} {width:.2} {height:.2} re f\n")
}

fn line(x1: f32, y1: f32, x2: f32, y2: f32) -> String {
    format!("{x1:.2} {y1:.2} m {x2:.2} {y2:.2} l S\n")
}

fn text(x: f32, y: f32, size: f32, value: &str) -> String {
    format!(
        "BT /F1 {size:.1} Tf {x:.2} {y:.2} Td ({}) Tj ET\n",
        escape(value)
    )
}

/// Build a PDF drawing of the scheduled Gantt chart.
#[must_use]
pub fn gantt_pdf(document: &Document, schedule: &Schedule) -> Vec<u8> {
    let days = schedule.day_span().max(1);
    let chart_width = PAGE_WIDTH - CHART_START - LEFT * 0.5;
    let day_width = (chart_width / days as f32).min(24.0);
    let ruler_top = TOP - 12.0;
    let step = ruler_step(days);
    let tick_count = days / step as i64;
    let page_count = document.tasks.len().div_ceil(ROWS_PER_PAGE).max(1);
    let mut pages = Vec::with_capacity(page_count);

    for page_index in 0..page_count {
        let page_start = page_index * ROWS_PER_PAGE;
        let page_end = (page_start + ROWS_PER_PAGE).min(document.tasks.len());
        let mut content = String::new();

        let _ = write!(content, "{}", color(0.09, 0.10, 0.13));
        let _ = write!(content, "{}", fill_rect(0.0, 0.0, PAGE_WIDTH, PAGE_HEIGHT));
        let _ = write!(content, "{}", color(0.92, 0.93, 0.96));
        let _ = write!(
            content,
            "{}",
            text(LEFT, 574.0, 18.0, &fit_text(&document.title, 90))
        );
        let summary = format!(
            "{} tasks  |  {} day span  |  total cost {:.2}  |  page {}/{}",
            document.tasks.len(),
            schedule.day_span(),
            document.total_cost(),
            page_index + 1,
            page_count
        );
        let _ = write!(content, "{}", text(LEFT, 558.0, 10.0, &summary));

        // Ruler ticks and labels. Counting ticks instead of incrementing an
        // inclusive i64 range keeps even extreme schedules bounded.
        let _ = write!(content, "{}", color(0.55, 0.58, 0.65));
        for tick in 0..=tick_count {
            let offset = tick * step as i64;
            let day = schedule.first_day().saturating_add(offset);
            let x = day_x(offset, CHART_START, day_width);
            let _ = write!(content, "{}", line(x, ruler_top - 6.0, x, ruler_top));
            let label = document.calendar.label_for_day(day);
            let _ = write!(content, "{}", text(x + 2.0, ruler_top + 10.0, 7.0, &label));
        }

        // Rows, bars and milestones for this page.
        for (local_row, task) in document.tasks[page_start..page_end].iter().enumerate() {
            let center_y = TOP - ROW_STEP * (local_row as f32 + 1.0);
            let _ = write!(
                content,
                "{}",
                text(LEFT, center_y - 3.0, 9.0, &fit_text(&task.name, 30))
            );
            let Some(timing) = schedule.task(task.id) else {
                continue;
            };
            let (left, top, right, bottom) = bar_rect(
                schedule.day_offset(timing.earliest_start),
                schedule.day_offset(timing.earliest_finish),
                CHART_START,
                center_y,
                day_width,
            );
            let (red, green, blue) = if timing.is_critical() {
                (0.91, 0.34, 0.25)
            } else {
                (0.42, 0.66, 0.96)
            };
            let _ = write!(content, "{}", color(red, green, blue));
            if task.milestone {
                let mid = (left + right) / 2.0;
                let half = 6.0;
                let cy = (top + bottom) / 2.0;
                let _ = writeln!(
                    content,
                    "{mid:.2} {cyplus:.2} m {xplus:.2} {cy:.2} l {mid:.2} {cyminus:.2} l {xminus:.2} {cy:.2} l f",
                    cyplus = cy + half,
                    xplus = mid + half,
                    cyminus = cy - half,
                    xminus = mid - half,
                );
            } else {
                let _ = write!(
                    content,
                    "{}",
                    fill_rect(left, top, right - left, bottom - top)
                );
                if task.progress > 0.0 {
                    let _ = write!(content, "{}", color(0.33, 0.82, 0.52));
                    let width = (right - left) * task.progress.clamp(0.0, 1.0);
                    let _ = write!(content, "{}", fill_rect(left, top, width, bottom - top));
                }
            }
        }

        // Dependency arrows whose two endpoints are on this page.
        let _ = write!(content, "0.55 0.58 0.65 RG\n1 w\n");
        for (local_row, task) in document.tasks[page_start..page_end].iter().enumerate() {
            let Some(timing) = schedule.task(task.id) else {
                continue;
            };
            let center_y = TOP - ROW_STEP * (local_row as f32 + 1.0);
            let end_x = day_x(
                schedule.day_offset(timing.earliest_start),
                CHART_START,
                day_width,
            );
            for dependency in &task.dependencies {
                let Some(relative_parent) = document.tasks[page_start..page_end]
                    .iter()
                    .position(|item| item.id == *dependency)
                else {
                    continue;
                };
                let Some(parent) = schedule.task(*dependency) else {
                    continue;
                };
                let parent_y = TOP - ROW_STEP * (relative_parent as f32 + 1.0);
                let from = (
                    day_x(
                        schedule.day_offset(parent.earliest_finish),
                        CHART_START,
                        day_width,
                    ),
                    parent_y,
                );
                let to = (end_x, center_y);
                let points = arrow_points(from, to);
                for pair in points.windows(2) {
                    let _ = write!(
                        content,
                        "{}",
                        line(pair[0].0, pair[0].1, pair[1].0, pair[1].1)
                    );
                }
                let _ = write!(content, "{}", line(to.0 - 6.0, to.1 + 3.0, to.0, to.1));
                let _ = write!(content, "{}", line(to.0 - 6.0, to.1 - 3.0, to.0, to.1));
            }
        }

        let _ = write!(
            content,
            "{}",
            text(LEFT, 24.0, 8.0, "ProjectCraft - exported Gantt chart")
        );
        pages.push(content);
    }

    build_pdf(&pages)
}

fn ruler_step(days: i64) -> usize {
    let preferred = if days > 60 {
        10
    } else if days > 20 {
        5
    } else {
        1
    };
    let minimum_step = ((i128::from(days) + i128::from(MAX_RULER_TICKS) - 1)
        / i128::from(MAX_RULER_TICKS))
    .clamp(1, usize::MAX as i128) as usize;
    preferred.max(minimum_step)
}

fn build_pdf(contents: &[String]) -> Vec<u8> {
    let page_count = contents.len();
    let font_object = 3 + page_count * 2;
    let kids = (0..page_count)
        .map(|index| format!("{} 0 R", 3 + index * 2))
        .collect::<Vec<_>>()
        .join(" ");
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        format!("<< /Type /Pages /Kids [{kids}] /Count {page_count} >>"),
    ];
    for (index, content) in contents.iter().enumerate() {
        let page_object = 3 + index * 2;
        let content_object = page_object + 1;
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PAGE_WIDTH:.0} {PAGE_HEIGHT:.0}] \
             /Resources << /Font << /F1 {font_object} 0 R >> >> /Contents {content_object} 0 R >>"
        ));
        objects.push(format!(
            "<< /Length {} >>\nstream\n{content}endstream",
            content.len()
        ));
    }
    objects.push(
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_owned(),
    );

    let mut output = Vec::new();
    output.extend_from_slice(b"%PDF-1.4\n");
    let mut offsets = Vec::with_capacity(objects.len());
    for (index, object) in objects.iter().enumerate() {
        offsets.push(output.len());
        output.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let xref_offset = output.len();
    let _ = write!(
        output_string(&mut output),
        "xref\n0 {}\n0000000000 65535 f \n",
        objects.len() + 1
    );
    for offset in &offsets {
        let _ = writeln!(output_string(&mut output), "{offset:010} 00000 n ");
    }
    let _ = write!(
        output_string(&mut output),
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n",
        objects.len() + 1
    );
    output
}

/// Helper allowing `write!` directly into a byte vector.
fn output_string(buffer: &mut Vec<u8>) -> ByteWriter<'_> {
    ByteWriter(buffer)
}

struct ByteWriter<'a>(&'a mut Vec<u8>);

impl std::fmt::Write for ByteWriter<'_> {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        self.0.extend_from_slice(text.as_bytes());
        Ok(())
    }
}

/// Write the Gantt PDF to `path` atomically.
///
/// # Errors
///
/// Returns a message naming the path when the file cannot be written.
pub fn export_pdf(
    path: &std::path::Path,
    document: &Document,
    schedule: &Schedule,
) -> Result<(), String> {
    craftkit_io::atomic::write_atomic(path, &gantt_pdf(document, schedule))
        .map_err(|error| format!("Could not write {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Task;
    use crate::schedule::schedule;

    fn assert_valid_xref(text: &str) {
        let start = text.rfind("startxref").expect("startxref") + "startxref\n".len();
        let offset: usize = text[start..]
            .lines()
            .next()
            .expect("offset")
            .parse()
            .expect("number");
        assert!(text[offset..].starts_with("xref"));
        let mut lines = text[offset..].lines();
        assert_eq!(lines.next(), Some("xref"));
        let count_line = lines.next().expect("xref count");
        let object_count: usize = count_line
            .split_whitespace()
            .nth(1)
            .expect("object count")
            .parse()
            .expect("number");
        assert_eq!(lines.next(), Some("0000000000 65535 f "));
        for object in 1..object_count {
            let object_offset: usize = lines
                .next()
                .expect("object offset")
                .split_whitespace()
                .next()
                .expect("offset")
                .parse()
                .expect("number");
            assert!(
                text[object_offset..].starts_with(&format!("{object} 0 obj")),
                "invalid xref entry for object {object}"
            );
        }
    }

    #[test]
    fn pdf_has_a_valid_structure() {
        let document = Document::sample();
        let result = schedule(&document).expect("sample");
        let bytes = gantt_pdf(&document, &result);
        let text = String::from_utf8(bytes.clone()).expect("pure ASCII PDF");
        assert!(text.starts_with("%PDF-1.4"));
        assert!(text.contains("/Type /Page "));
        assert!(text.contains("Requirements"));
        assert!(text.contains("startxref"));
        assert!(text.trim_end().ends_with("%%EOF"));
        assert_valid_xref(&text);
    }

    #[test]
    fn pdf_escapes_problematic_characters_and_preserves_latin1() {
        assert_eq!(escape("a(b)\\c"), "a\\(b\\)\\\\c");
        assert_eq!(escape("café"), "caf\\351");
        let document = Document {
            title: "Test (draft)".into(),
            ..Document::default()
        };
        let result = schedule(&document).expect("empty");
        let bytes = gantt_pdf(&document, &result);
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("Test \\(draft\\)"));
    }

    #[test]
    fn pdf_paginates_instead_of_dropping_tasks() {
        let tasks: Vec<Task> = (1..=60)
            .map(|id| Task {
                id,
                name: format!("Task {id:02}"),
                start_day: 0,
                duration_days: 1,
                dependencies: Vec::new(),
                progress: 0.0,
                milestone: false,
                notes: String::new(),
                resource: String::new(),
                cost: 0.0,
            })
            .collect();
        let document = Document {
            title: "Long plan".into(),
            next_id: 61,
            tasks,
            ..Document::default()
        };
        let result = schedule(&document).expect("schedule");
        let bytes = gantt_pdf(&document, &result);
        let text = String::from_utf8(bytes).expect("ASCII PDF");
        assert!(text.contains("/Count 3"));
        assert_eq!(text.matches("/Type /Page ").count(), 3);
        assert!(text.contains("Task 01"));
        assert!(text.contains("Task 60"));
        assert_valid_xref(&text);
    }

    #[test]
    fn pdf_draws_signed_days_from_the_first_scheduled_day() {
        let mut document = Document::default();
        document.tasks.push(Task {
            id: 1,
            name: "Before epoch".into(),
            start_day: -5,
            duration_days: 2,
            dependencies: Vec::new(),
            progress: 0.0,
            milestone: false,
            notes: String::new(),
            resource: String::new(),
            cost: 0.0,
        });
        let result = schedule(&document).expect("schedule");
        let text = String::from_utf8(gantt_pdf(&document, &result)).expect("ASCII PDF");
        assert!(text.contains("D-5"));
        assert!(text.contains("Before epoch"));
    }

    #[test]
    fn extreme_duration_keeps_the_pdf_ruler_bounded() {
        let mut document = Document::default();
        document.tasks.push(Task {
            id: 1,
            name: "Huge".into(),
            start_day: 0,
            duration_days: i64::MAX,
            dependencies: Vec::new(),
            progress: 0.0,
            milestone: false,
            notes: String::new(),
            resource: String::new(),
            cost: 0.0,
        });
        let result = schedule(&document).expect("schedule");
        assert!(gantt_pdf(&document, &result).starts_with(b"%PDF-1.4"));
    }

    #[test]
    fn export_writes_a_pdf_file() {
        let path = std::env::temp_dir().join(format!("projectcraft-pdf-{}", std::process::id()));
        let document = Document::sample();
        let result = schedule(&document).expect("sample");
        export_pdf(&path, &document, &result).expect("export");
        let bytes = std::fs::read(&path).expect("read");
        assert_eq!(&bytes[..8], b"%PDF-1.4");
        let _ = std::fs::remove_file(&path);
    }
}
