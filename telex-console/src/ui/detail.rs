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
            Span::raw(sanitize(s)),
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

    // Delivery/consume badge. In the local-exchange model a message is fanned out to one delivery
    // row per recipient at insert; `consumed_at_ms` is set when a row is consumed — by a waiter
    // handoff, a cc auto-consume, or a terminal disposition. The daemon consume path records no
    // occupant (that column is written only by the legacy `mark_delivered` path), so we can't
    // reliably single out a real waiter handoff: we show the honest consumed/pending split and
    // surface an `occupant` only on the rare row that carries one.
    lines.push(Line::from(""));
    let (consumed, pending): (Vec<&DeliveryRow>, Vec<&DeliveryRow>) =
        dels.iter().partition(|d| d.consumed_at_ms.is_some());
    if !consumed.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            format!("{} consumed", theme::delivered_symbol(true)),
            Style::default().fg(theme::delivered_color(true)),
        )]));
        for d in &consumed {
            lines.push(delivery_line(d));
        }
    }
    if !pending.is_empty() || consumed.is_empty() {
        let label = if pending.is_empty() {
            " pending (no delivery rows yet)".to_string()
        } else {
            format!(" {} pending", pending.len())
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("{}{}", theme::delivered_symbol(false), label),
                Style::default().fg(theme::delivered_color(false)),
            ),
            Span::styled(
                " (queued; not yet consumed)",
                Style::default().fg(Color::DarkGray),
            ),
        ]));
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

/// Push plain text into `lines`, hard word-wrapping to `wrap` width when set. In both modes the
/// text is sanitized of terminal-control characters so raw content bytes can't drive the terminal.
fn push_text(lines: &mut Vec<Line<'static>>, text: &str, wrap: Option<usize>) {
    match wrap {
        Some(w) => {
            for seg in wrap_text(text, w) {
                lines.push(Line::from(seg));
            }
        }
        None => {
            for raw in text.lines() {
                lines.push(Line::from(sanitize(raw)));
            }
        }
    }
}

/// Strip terminal-control characters from a display string: tabs become spaces and other control
/// characters (ESC, C0/C1, DEL) are dropped, so untrusted message bytes can never emit terminal
/// control sequences. (ratatui renders into a cell buffer, but this is cheap defense-in-depth.)
pub(crate) fn sanitize(line: &str) -> String {
    line.chars()
        .map(|c| if c == '\t' { ' ' } else { c })
        .filter(|c| !c.is_control())
        .collect()
}

/// Display width of a `char` in terminal columns (0 for combining/zero-width).
fn char_width(c: char) -> usize {
    unicode_width::UnicodeWidthChar::width(c).unwrap_or(0)
}

/// Display width of a string in terminal columns.
fn str_width(s: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(s)
}

/// Word-wrap `text` (honoring existing newlines) to `width` **display columns**, hard-breaking any
/// single word wider than the width. Sanitizes control characters first. Measuring by display
/// width (not `char` count) keeps "one returned line == one rendered row" for wide/CJK/emoji, which
/// the reader relies on for exact scroll bounds.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    for raw in text.split('\n') {
        let raw = sanitize(raw);
        if raw.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut cur = String::new();
        let mut cur_w = 0usize;
        for word in raw.split(' ') {
            let wlen = str_width(word);
            if wlen > width {
                if cur_w > 0 {
                    out.push(std::mem::take(&mut cur));
                    cur_w = 0;
                }
                let mut chunk = String::new();
                let mut cw = 0usize;
                for ch in word.chars() {
                    let cwch = char_width(ch);
                    if cw + cwch > width && cw > 0 {
                        out.push(std::mem::take(&mut chunk));
                        cw = 0;
                    }
                    chunk.push(ch);
                    cw += cwch;
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
    let mut label = format!("  {}", d.recipient);
    if let Some(occ) = d.occupant.as_deref() {
        label.push_str(&format!(" → {occ}"));
    }
    let mut spans = vec![Span::raw(label)];
    let ts = d.consumed_at_ms.unwrap_or(d.delivered_at_ms);
    spans.push(Span::styled(
        format!(" @{}", theme::hms(ts)),
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
