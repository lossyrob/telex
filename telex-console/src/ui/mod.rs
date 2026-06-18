//! Rendering. All functions here are read-only over [`AppState`] and draw into a
//! ratatui `Frame`, so they can be exercised with a `TestBackend` in unit tests.

mod addresses;
mod detail;
mod feed;
mod theme;
mod thread;

use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{AppState, Mode, View};

/// Minimum terminal size below which we show a notice instead of a cramped UI.
const MIN_WIDTH: u16 = 50;
const MIN_HEIGHT: u16 = 8;

/// Top-level render entry point.
pub fn render(f: &mut Frame, st: &AppState) {
    let area = f.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        let msg = Paragraph::new(format!(
            "terminal too small ({}x{}) — need at least {MIN_WIDTH}x{MIN_HEIGHT}",
            area.width, area.height
        ));
        f.render_widget(msg, area);
        return;
    }

    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(area);

    render_header(f, chunks[0], st);
    match st.view {
        View::Feed => feed::render(f, chunks[1], st),
        View::Addresses => addresses::render(f, chunks[1], st),
        View::Thread => thread::render(f, chunks[1], st),
    }
    render_footer(f, chunks[2], st);
}

fn render_header(f: &mut Frame, area: ratatui::layout::Rect, st: &AppState) {
    let view = match st.view {
        View::Feed => "FEED",
        View::Addresses => "ADDRESSES",
        View::Thread => "THREAD",
    };
    let (live_txt, live_color) = if st.tailing {
        ("● LIVE", Color::Green)
    } else {
        ("⏸ PAUSED (t resumes)", Color::Yellow)
    };
    let filter_txt = st
        .filter
        .as_deref()
        .map(|f| format!(" │ filter:{f}"))
        .unwrap_or_default();

    let mut spans = vec![
        Span::styled(
            " telex-console ",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(
            "│ {}({}) │ {view} │ ",
            st.backend_name, st.backend_kind
        )),
        Span::styled(live_txt, Style::default().fg(live_color)),
        Span::raw(format!(
            "{filter_txt} │ msgs:{} addrs:{}",
            st.feed.len(),
            st.addresses.len()
        )),
    ];
    if let Some(err) = &st.status {
        spans.push(Span::styled(
            format!("  ⚠ {err}"),
            Style::default().fg(Color::Red),
        ));
    }
    let header = Paragraph::new(Line::from(spans))
        .style(Style::default().bg(Color::Rgb(30, 30, 40)).fg(Color::White));
    f.render_widget(header, area);
}

fn render_footer(f: &mut Frame, area: ratatui::layout::Rect, st: &AppState) {
    let line = match &st.mode {
        Mode::Filter(buf) => Line::from(vec![
            Span::styled("filter> ", Style::default().fg(Color::Cyan)),
            Span::raw(buf.clone()),
            Span::styled("▏", Style::default().fg(Color::Cyan)),
            Span::styled(
                "   (Enter apply · Esc cancel)",
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Mode::Normal => {
            // Surface the tail state in the key hint, and a prominent resume hint when paused
            // so it's discoverable that new messages are still arriving but auto-scroll is off.
            let tail_hint = if st.view == View::Feed && !st.tailing {
                Span::styled(
                    " t/G resume-tail ",
                    Style::default()
                        .fg(ratatui::style::Color::Black)
                        .bg(ratatui::style::Color::Yellow),
                )
            } else {
                Span::styled(" t tail ", Style::default().fg(Color::DarkGray))
            };
            Line::from(vec![
                Span::styled(" q quit · Tab view ·", Style::default().fg(Color::DarkGray)),
                tail_hint,
                Span::styled(
                    "· f filter · Enter thread · j/k move · ←/→ column · g/G ends · Esc back",
                    Style::default().fg(Color::DarkGray),
                ),
            ])
        }
    };
    f.render_widget(Paragraph::new(line), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Backfill;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use telex::model::MessageRow;

    fn render_to_string(width: u16, height: u16, st: &AppState) -> String {
        let mut term = Terminal::new(TestBackend::new(width, height)).unwrap();
        term.draw(|f| render(f, st)).unwrap();
        let buf = term.backend().buffer().clone();
        let mut s = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                s.push_str(buf[(x, y)].symbol());
            }
            s.push('\n');
        }
        s
    }

    fn state() -> AppState {
        AppState::new(
            "default".into(),
            "sqlite".into(),
            None,
            Backfill::Recent(200),
        )
    }

    fn msg(id: i64, from: &str, to: &str, subject: &str) -> MessageRow {
        MessageRow {
            id,
            thread_id: id,
            parent_id: None,
            from_addr: Some(from.into()),
            to_addr: to.into(),
            cc: None,
            kind: "note".into(),
            attention: "interrupt".into(),
            requires_disposition: true,
            subject: Some(subject.into()),
            body: "body".into(),
            metadata: None,
            sent_at_ms: 0,
            created_at_ms: 0,
        }
    }

    #[test]
    fn empty_feed_renders_header_and_placeholder() {
        let out = render_to_string(80, 20, &state());
        assert!(out.contains("telex-console"));
        assert!(out.contains("FEED"));
        assert!(out.contains("LIVE"));
        assert!(out.contains("waiting for messages"));
    }

    #[test]
    fn too_small_terminal_shows_notice() {
        let out = render_to_string(20, 5, &state());
        assert!(out.contains("too small"));
    }

    #[test]
    fn populated_feed_shows_message() {
        let mut st = state();
        st.apply_feed(vec![msg(1, "impl-215", "orch", "PR ready")]);
        let out = render_to_string(100, 20, &st);
        assert!(out.contains("impl-215"));
        assert!(out.contains("orch"));
        assert!(out.contains("PR ready"));
    }
}
