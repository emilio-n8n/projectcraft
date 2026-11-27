//! Pure geometry helpers for the Gantt canvas.
//!
//! Keeping the maths here makes zoom, hit-testing, dragging and dependency
//! arrows testable without opening a window.

/// Width of the task-name gutter, in pixels.
pub const LABEL_WIDTH: f32 = 175.0;
/// Height of one task row.
pub const ROW_HEIGHT: f32 = 22.0;
/// Height of the day ruler.
pub const HEADER_HEIGHT: f32 = 26.0;

/// Gantt time scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zoom {
    /// One day at full width.
    Day,
    /// Compressed scale, labelled by week.
    Week,
    /// Compressed scale, labelled by month.
    Month,
    /// Fit the whole project into the available width.
    Fit,
}

impl Zoom {
    /// All zoom modes, in menu order.
    #[must_use]
    pub const fn all() -> [Self; 4] {
        [Self::Day, Self::Week, Self::Month, Self::Fit]
    }

    /// Short label shown in the toolbar.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Day => "Day",
            Self::Week => "Week",
            Self::Month => "Month",
            Self::Fit => "Fit",
        }
    }

    /// Pixels per project day for this zoom level.
    #[must_use]
    pub fn day_width(self, days: i64, available_width: f32) -> f32 {
        let drawn_width = (available_width - LABEL_WIDTH - 24.0).max(1.0);
        match self {
            Self::Day => 18.0,
            Self::Week => 7.0,
            Self::Month => 2.5,
            Self::Fit => (drawn_width / days.max(1) as f32).max(1.0),
        }
    }

    /// Day interval between ruler labels.
    ///
    /// Very long plans use a coarser step so drawing the ruler remains bounded.
    #[must_use]
    pub fn ruler_step(self, days: i64) -> usize {
        let preferred = match self {
            Self::Day => 1,
            Self::Week => 7,
            Self::Month => 30,
            Self::Fit => {
                if days > 90 {
                    30
                } else if days > 30 {
                    7
                } else {
                    1
                }
            }
        };
        if days <= 0 {
            return preferred;
        }
        let maximum_ticks = 120_i128;
        let minimum_step = ((i128::from(days) + maximum_ticks - 1) / maximum_ticks)
            .clamp(1, usize::MAX as i128) as usize;
        preferred.max(minimum_step)
    }
}

/// Horizontal pixel position of a project day.
#[must_use]
pub fn day_x(day: i64, origin_x: f32, day_width: f32) -> f32 {
    origin_x + day as f32 * day_width
}

/// Bar geometry `(left, top, right, bottom)` for a task's earliest dates.
#[must_use]
pub fn bar_rect(
    earliest_start: i64,
    earliest_finish: i64,
    origin_x: f32,
    center_y: f32,
    day_width: f32,
) -> (f32, f32, f32, f32) {
    let left = day_x(earliest_start, origin_x, day_width);
    let right = day_x(earliest_finish, origin_x, day_width).max(left + 2.0);
    (left, center_y - 7.0, right, center_y + 7.0)
}

/// Whole-day shift produced by dragging a bar.
#[must_use]
pub fn drag_delta_days(start_x: f32, current_x: f32, day_width: f32) -> i64 {
    if !day_width.is_finite() || day_width <= 0.0 || !start_x.is_finite() || !current_x.is_finite()
    {
        return 0;
    }
    ((current_x - start_x) / day_width).round() as i64
}

/// New release day for a dragged bar.
///
/// The visible bar starts at the CPM earliest start, not necessarily at the
/// authored release day. Basing the gesture on the latter makes dependent
/// tasks appear not to move.
#[must_use]
pub fn drag_target_start(earliest_start: i64, delta: i64) -> i64 {
    earliest_start.saturating_add(delta)
}

/// Orthogonal dependency arrow from a predecessor to a successor.
///
/// Returns four points: source, elbow start, elbow end and arrow tip.
#[must_use]
pub fn arrow_points(from: (f32, f32), to: (f32, f32)) -> [(f32, f32); 4] {
    let elbow = (from.0 + 10.0).min(to.0 - 2.0);
    [from, (elbow, from.1), (elbow, to.1), (to.0 - 4.0, to.1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_widths_are_ordered_and_fit_the_canvas() {
        let days = 30;
        let available = 900.0;
        let day = Zoom::Day.day_width(days, available);
        let week = Zoom::Week.day_width(days, available);
        let month = Zoom::Month.day_width(days, available);
        assert!(day > week && week > month);
        assert!(Zoom::Fit.day_width(days, available) > 0.0);
        assert!(Zoom::Fit.day_width(days, available) > day);
        assert!(Zoom::Fit.day_width(1_000, available) < Zoom::Fit.day_width(10, available));
        assert_eq!(Zoom::Day.ruler_step(days), 1);
        assert_eq!(Zoom::Week.ruler_step(days), 7);
        assert_eq!(Zoom::Month.ruler_step(days), 30);
    }

    #[test]
    fn bar_rect_uses_the_earliest_dates() {
        let (left, top, right, bottom) = bar_rect(3, 7, 100.0, 50.0, 20.0);
        assert_eq!((left, right), (160.0, 240.0));
        assert_eq!((top, bottom), (43.0, 57.0));
        let (left, _, right, _) = bar_rect(5, 5, 0.0, 0.0, 10.0);
        assert_eq!((left, right), (50.0, 52.0)); // minimum visible width
    }

    #[test]
    fn drag_delta_rounds_to_whole_days() {
        assert_eq!(drag_delta_days(100.0, 141.0, 20.0), 2);
        assert_eq!(drag_delta_days(100.0, 89.0, 20.0), -1);
        assert_eq!(drag_delta_days(100.0, 100.0, 20.0), 0);
        assert_eq!(drag_delta_days(100.0, 120.0, 0.0), 0);
        assert_eq!(drag_delta_days(f32::NAN, 120.0, 20.0), 0);
    }

    #[test]
    fn dragging_uses_the_visible_cpm_start() {
        assert_eq!(drag_target_start(5, 2), 7);
        assert_eq!(drag_target_start(5, -3), 2);
        assert_eq!(drag_target_start(i64::MAX, 1), i64::MAX);
        assert_eq!(drag_target_start(i64::MIN, -1), i64::MIN);
    }

    #[test]
    fn ruler_steps_stay_bounded_for_extreme_plans() {
        for zoom in Zoom::all() {
            let step = zoom.ruler_step(i64::MAX);
            assert!(step > 0);
            assert!(i128::from(i64::MAX) / step as i128 <= 120);
        }
    }

    #[test]
    fn arrow_points_route_around_the_successor() {
        let points = arrow_points((100.0, 20.0), (300.0, 60.0));
        assert_eq!(points[0], (100.0, 20.0));
        assert_eq!(points[1], (110.0, 20.0));
        assert_eq!(points[2], (110.0, 60.0));
        assert_eq!(points[3], (296.0, 60.0));
    }
}
