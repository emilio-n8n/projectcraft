//! Minimal PDF export of the Gantt chart.
//!
//! Writes a small, valid PDF 1.4 document with vector bars, dependency arrows
//! and text. No third-party PDF crate or external tool is required.

use std::fmt::Write as _;

use crate::document::Document;
use crate::gantt::{LABEL_WIDTH, arrow_points, bar_rect, day_x};
use crate::schedule::Schedule;

const PAGE_WIDTH: f32 = 842.0; // A4 landscape
const PAGE_HEIGHT: f32 = 595.0;
const LEFT: f32 = 48.0;
const TOP: f32 = 540.0;
const ROW_STEP: f32 = 20.0;
const CHART_START: f32 = LEFT + LABEL_WIDTH * 0.8;

/// Escape a string for a PDF literal.
fn escape(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '(' => output.push_str("\\("),
            ')' => output.push_str("\\)"),
            '\n' | '\r' => output.push(' '),
            other if other.is_ascii() => output.push(other),
            _ => output.push('?'),
        }
    }
    output
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
    let days = schedule.project_duration.max(1);
    let chart_width = PAGE_WIDTH - CHART_START - LEFT * 0.5;
    let day_width = (chart_width / days as f32).min(24.0);
    let ruler_top = TOP - 12.0;
    let mut content = String::new();

    let _ = write!(content, "{}", color(0.09, 0.10, 0.13),);
    let _ = write!(content, "{}", fill_rect(0.0, 0.0, PAGE_WIDTH, PAGE_HEIGHT));
    let _ = write!(content, "{}", color(0.92, 0.93, 0.96));
    let _ = write!(content, "{}", text(LEFT, 574.0, 18.0, &document.title));
    let summary = format!(
        "{} tasks  |  {} days  |  total cost {:.2}",
        document.tasks.len(),
        schedule.project_duration,
        document.total_cost()
    );
    let _ = write!(content, "{}", text(LEFT, 558.0, 10.0, &summary));

    // Ruler ticks and labels.
    let step = if days > 60 {
        10
    } else if days > 20 {
        5
    } else {
        1
    };
    let _ = write!(content, "{}", color(0.55, 0.58, 0.65));
    for day in (0..=days).step_by(step) {
        let x = day_x(day, CHART_START, day_width);
        let _ = write!(content, "{}", line(x, ruler_top - 6.0, x, ruler_top));
        let label = document.calendar.label_for_day(day);
        let _ = write!(content, "{}", text(x + 2.0, ruler_top + 10.0, 7.0, &label));
    }

    // Rows, bars and milestones.
    for (row, task) in document.tasks.iter().enumerate() {
        let center_y = TOP - ROW_STEP * (row as f32 + 1.0);
        if center_y < 40.0 {
            break;
        }
        let _ = write!(content, "{}", text(LEFT, center_y - 3.0, 9.0, &task.name));
        let Some(timing) = schedule.task(task.id) else {
            continue;
        };
        let (left, top, right, bottom) = bar_rect(
            timing.earliest_start,
            timing.earliest_finish,
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

    // Dependency arrows.
    let _ = write!(content, "0.55 0.58 0.65 RG\n1 w\n");
    for (row, task) in document.tasks.iter().enumerate() {
        let Some(timing) = schedule.task(task.id) else {
            continue;
        };
        let center_y = TOP - ROW_STEP * (row as f32 + 1.0);
        let end_x = day_x(timing.earliest_finish, CHART_START, day_width);
        for dependency in &task.dependencies {
            let Some(parent_index) = document
                .tasks
                .iter()
                .position(|item| item.id == *dependency)
            else {
                continue;
            };
            let Some(parent) = schedule.task(*dependency) else {
                continue;
            };
            let parent_y = TOP - ROW_STEP * (parent_index as f32 + 1.0);
            let from = (
                day_x(parent.earliest_finish, CHART_START, day_width),
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

    build_pdf(&content)
}

fn build_pdf(content: &str) -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PAGE_WIDTH:.0} {PAGE_HEIGHT:.0}] \
             /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>"
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_owned(),
        format!(
            "<< /Length {} >>\nstream\n{content}endstream",
            content.len()
        ),
    ];

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

/// Write the Gantt PDF to `path`.
///
/// # Errors
///
/// Returns a message naming the path when the file cannot be written.
pub fn export_pdf(
    path: &std::path::Path,
    document: &Document,
    schedule: &Schedule,
) -> Result<(), String> {
    std::fs::write(path, gantt_pdf(document, schedule))
        .map_err(|error| format!("Could not write {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schedule::schedule;

    #[test]
    fn pdf_has_a_valid_structure() {
        let document = Document::sample();
        let result = schedule(&document).expect("sample");
        let bytes = gantt_pdf(&document, &result);
        let text = String::from_utf8(bytes.clone()).expect("pure ASCII PDF");
        assert!(text.starts_with("%PDF-1.4"));
        assert!(text.contains("/Type /Page"));
        assert!(text.contains("Requirements"));
        assert!(text.contains("startxref"));
        assert!(text.trim_end().ends_with("%%EOF"));
        let start = text.rfind("startxref").expect("startxref") + "startxref\n".len();
        let offset: usize = text[start..]
            .lines()
            .next()
            .expect("offset")
            .parse()
            .expect("number");
        assert!(text[offset..].starts_with("xref"));
    }

    #[test]
    fn pdf_escapes_problematic_characters() {
        assert_eq!(escape("a(b)\\c"), "a\\(b\\)\\\\c");
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
