//! The shared detail content: full headers, body, pretty-printed metadata, delivery
//! state, and disposition history for a single message. Used both by the side detail
//! pane (soft-wrapped by ratatui) and the full-screen reader (pre-wrapped + scrollable).

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::AppState;
use crate::ui::theme;
use telex::model::{DeliveryRow, DispositionRow, MessageRow};

/// Render the side detail pane for `msg` (or a placeholder when nothing is selected).
/// Long content is soft-wrapped by ratatui and clipped to the pane; the full-screen
/// reader (`o` / Enter-in-thread) is the way to read past the bottom.
pub fn render(f: &mut Frame, area: Rect, st: &AppState, msg: Option<&MessageRow>) {
    let block = Block::default().borders(Borders::ALL).title(" detail ");
    let Some(m) = msg else {
        f.render_widget(
            Paragraph::new("(no message selected)")
                .style(Style::default().fg(Color::DarkGray))
                .block(block),
            area,
        );
        return;
    };

    let (disps, dels) = detail_rows(st, m.id);
    let lines = build_lines(m, disps, dels, None);
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(block),
        area,
    );
}

/// The dispositions and deliveries cached for message `id` (empty slices if not loaded).
pub(crate) fn detail_rows(st: &AppState, id: i64) -> (&[DispositionRow], &[DeliveryRow]) {
    let disps = st
        .detail_disp
        .as_ref()
        .filter(|(i, _)| *i == id)
        .map(|(_, d)| d.as_slice())
        .unwrap_or(&[]);
    let dels = st
        .detail_deliv
        .as_ref()
        .filter(|(i, _)| *i == id)
        .map(|(_, d)| d.as_slice())
        .unwrap_or(&[]);
    (disps, dels)
}

/// Build the detail content for a message. When `wrap` is `Some(width)`, the body and
/// metadata are hard word-wrapped to that width so the result has one rendered row per
/// `Line` (the reader relies on this for exact scroll bounds). When `None`, body and
/// metadata lines are emitted verbatim for ratatui to soft-wrap.
pub(crate) fn build_lines(
    m: &MessageRow,
    disps: &[DispositionRow],
    dels: &[DeliveryRow],
    wrap: Option<usize>,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled(
            format!("#{}", m.id),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!("[{}]", m.attention),
            Style::default().fg(theme::attn_color(&m.attention)),
        ),
        Span::raw(format!("  {}", m.kind)),
        if m.requires_disposition {
            Span::styled("  *needs-disposition", Style::default().fg(Color::Red))
        } else {
            Span::raw("")
        },
    ]));
    lines.push(Line::from(format!(
        "from: {}",
        m.from_addr.as_deref().unwrap_or("?")
    )));
    lines.push(Line::from(format!("to:   {}", m.to_addr)));
    if let Some(cc) = &m.cc {
        lines.push(Line::from(format!("cc:   {cc}")));
    }
    lines.push(Line::from(format!(
        "sent: {}   thread #{}",
        theme::hms(m.sent_at_ms),
        m.thread_id
    )));
    if let Some(s) = &m.subject {
        lines.push(Line::from(vec![
            Span::styled("subject: ", Style::default().fg(Color::Gray)),
            Span::raw(s.clone()),
        ]));
    }

    lines.push(Line::from(""));
    push_text(&mut lines, &m.body, wrap);

    if let Some(meta) = &m.metadata {
        lines.push(Line::from(""));
        lines.push(Line::styled(
            "── metadata ──",
            Style::default().fg(Color::DarkGray),
        ));
        push_text(&mut lines, &pretty_json(meta), wrap);
    }

    // Delivery badge: delivered (reached a waiter) is distinct from dispositioned/acted-on.
    lines.push(Line::from(""));
    if dels.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} undelivered", theme::delivered_symbol(false)),
                Style::default().fg(theme::delivered_color(false)),
            ),
            Span::styled(
                " (queued; not yet handed to a waiter)",
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    } else {
        lines.push(Line::from(vec![Span::styled(
            format!("{} delivered", theme::delivered_symbol(true)),
            Style::default().fg(theme::delivered_color(true)),
        )]));
        for d in dels {
            lines.push(delivery_line(d));
        }
    }

    if !disps.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::styled(
            "── dispositions ──",
            Style::default().fg(Color::DarkGray),
        ));
        for d in disps {
            lines.push(disposition_line(d));
        }
    }

    lines
}

/// Push plain text into `lines`, hard word-wrapping to `wrap` width when set.
fn push_text(lines: &mut Vec<Line<'static>>, text: &str, wrap: Option<usize>) {
    match wrap {
        Some(w) => {
            for seg in wrap_text(text, w) {
                lines.push(Line::from(seg));
            }
        }
        None => {
            for raw in text.lines() {
                lines.push(Line::from(raw.to_string()));
            }
        }
    }
}

/// Word-wrap `text` (honoring existing newlines) to `width` columns, hard-breaking any
/// single word longer than the width. Width is measured in `char`s — close enough for the
/// mostly-ASCII operational text telex carries.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    for raw in text.split('\n') {
        if raw.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut cur = String::new();
        let mut cur_w = 0usize;
        for word in raw.split(' ') {
            let wlen = word.chars().count();
            if wlen > width {
                if cur_w > 0 {
                    out.push(std::mem::take(&mut cur));
                    cur_w = 0;
                }
                let mut chunk = String::new();
                let mut cw = 0usize;
                for ch in word.chars() {
                    chunk.push(ch);
                    cw += 1;
                    if cw == width {
                        out.push(std::mem::take(&mut chunk));
                        cw = 0;
                    }
                }
                if cw > 0 {
                    cur = chunk;
                    cur_w = cw;
                }
                continue;
            }
            let needed = if cur_w == 0 { wlen } else { cur_w + 1 + wlen };
            if needed > width {
                out.push(std::mem::take(&mut cur));
                cur.push_str(word);
                cur_w = wlen;
            } else {
                if cur_w > 0 {
                    cur.push(' ');
                    cur_w += 1;
                }
                cur.push_str(word);
                cur_w += wlen;
            }
        }
        out.push(cur);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

fn disposition_line(d: &DispositionRow) -> Line<'static> {
    let mut spans = vec![Span::styled(
        d.state.clone(),
        Style::default().fg(theme::disp_color(Some(&d.state))),
    )];
    if let Some(by) = &d.by_principal {
        spans.push(Span::raw(format!(" by {by}")));
    }
    spans.push(Span::raw(format!(" @{}", theme::hms(d.at_ms))));
    if let Some(note) = &d.note {
        spans.push(Span::styled(
            format!(" — {note}"),
            Style::default().fg(Color::Gray),
        ));
    }
    Line::from(spans)
}

fn delivery_line(d: &DeliveryRow) -> Line<'static> {
    let mut spans = vec![Span::raw(format!(
        "  → {}",
        d.occupant.as_deref().unwrap_or("?")
    ))];
    spans.push(Span::styled(
        format!(" @{}", theme::hms(d.delivered_at_ms)),
        Style::default().fg(Color::Gray),
    ));
    Line::from(spans)
}

/// Pretty-print a JSON metadata string; fall back to the raw text if it doesn't parse.
fn pretty_json(raw: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(v) => serde_json::to_string_pretty(&v).unwrap_or_else(|_| raw.to_string()),
        Err(_) => raw.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_respects_width_and_breaks_long_words() {
        let w = wrap_text("the quick brown fox", 9);
        assert!(w.iter().all(|l| l.chars().count() <= 9));
        assert!(w.len() >= 2);
        // a single over-long token is hard-broken into width-sized chunks
        let w2 = wrap_text("abcdefghij", 4);
        assert_eq!(w2, vec!["abcd", "efgh", "ij"]);
    }

    #[test]
    fn wrap_preserves_blank_lines() {
        let w = wrap_text("a\n\nb", 10);
        assert_eq!(w, vec!["a", "", "b"]);
    }
}
