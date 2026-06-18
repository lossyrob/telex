//! Visual theme: symbols and colors for attention levels, disposition states, and
//! occupancy, plus a small UTC time-of-day formatter for feed timestamps.

use ratatui::style::Color;

use crate::data::Occ;

/// One-character marker for an attention level.
pub fn attn_symbol(attention: &str) -> char {
    match attention {
        "interrupt" => '!',
        "next-checkpoint" => '^',
        "background" => '·',
        "fyi" => '°',
        _ => ' ',
    }
}

/// Color for an attention level (warmer = more urgent).
pub fn attn_color(attention: &str) -> Color {
    match attention {
        "interrupt" => Color::Red,
        "next-checkpoint" => Color::Yellow,
        "background" => Color::Gray,
        "fyi" => Color::DarkGray,
        _ => Color::Gray,
    }
}

/// Short label for a disposition state, or `-` when absent.
pub fn disp_label(disposition: Option<&str>) -> &str {
    disposition.unwrap_or("-")
}

/// Color for a disposition state.
pub fn disp_color(disposition: Option<&str>) -> Color {
    match disposition {
        Some("handled") | Some("closed") => Color::Green,
        Some("rejected") => Color::Red,
        Some("escalated") => Color::Magenta,
        Some("deferred") => Color::Yellow,
        Some("acknowledged") => Color::Cyan,
        _ => Color::DarkGray,
    }
}

/// Occupancy dot + color.
pub fn occ_symbol(occ: Occ) -> char {
    match occ {
        Occ::Live => '●',
        Occ::Idle => '○',
        Occ::Unknown => '?',
    }
}

pub fn occ_color(occ: Occ) -> Color {
    match occ {
        Occ::Live => Color::Green,
        Occ::Idle => Color::DarkGray,
        Occ::Unknown => Color::Yellow,
    }
}

/// Delivered-state badge symbol: a message either has a delivery record (reached a waiter)
/// or is still queued. "Delivered" is distinct from "dispositioned/acted-on".
pub fn delivered_symbol(delivered: bool) -> char {
    if delivered {
        '✓'
    } else {
        '⧗'
    }
}

pub fn delivered_color(delivered: bool) -> Color {
    if delivered {
        Color::Green
    } else {
        Color::Yellow
    }
}

/// Format an epoch-millisecond timestamp as a `HH:MM:SS` UTC time-of-day. Good enough
/// for a live feed without pulling in a date/time dependency.
pub fn hms_utc(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let s = secs.rem_euclid(60);
    let m = secs.div_euclid(60).rem_euclid(60);
    let h = secs.div_euclid(3600).rem_euclid(24);
    format!("{h:02}:{m:02}:{s:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hms_formats_utc_time_of_day() {
        // 1_000 ms = 00:00:01; 3_661_000 ms = 01:01:01.
        assert_eq!(hms_utc(1_000), "00:00:01");
        assert_eq!(hms_utc(3_661_000), "01:01:01");
    }
}
