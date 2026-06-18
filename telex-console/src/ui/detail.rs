//! The shared detail pane: full headers, body, pretty-printed metadata, and disposition
//! history for a single selected message.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::AppState;
use crate::ui::theme;
use telex::model::{DeliveryRow, DispositionRow, MessageRow};

/// Render the detail pane for `msg` (or a placeholder when nothing is selected).
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
        theme::hms_utc(m.sent_at_ms),
        m.thread_id
    )));
    if let Some(s) = &m.subject {
        lines.push(Line::from(vec![
            Span::styled("subject: ", Style::default().fg(Color::Gray)),
            Span::raw(s.clone()),
        ]));
    }

    lines.push(Line::from(""));
    for bl in m.body.lines() {
        lines.push(Line::from(bl.to_string()));
    }

    if let Some(meta) = &m.metadata {
        lines.push(Line::from(""));
        lines.push(Line::styled(
            "── metadata ──",
            Style::default().fg(Color::DarkGray),
        ));
        for bl in pretty_json(meta).lines() {
            lines.push(Line::from(bl.to_string()));
        }
    }

    let disps = st
        .detail_disp
        .as_ref()
        .filter(|(id, _)| *id == m.id)
        .map(|(_, d)| d.as_slice())
        .unwrap_or(&[]);
    let dels = st
        .detail_deliv
        .as_ref()
        .filter(|(id, _)| *id == m.id)
        .map(|(_, d)| d.as_slice())
        .unwrap_or(&[]);

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

    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(block),
        area,
    );
}

fn disposition_line(d: &DispositionRow) -> Line<'static> {
    let mut spans = vec![Span::styled(
        d.state.clone(),
        Style::default().fg(theme::disp_color(Some(&d.state))),
    )];
    if let Some(by) = &d.by_principal {
        spans.push(Span::raw(format!(" by {by}")));
    }
    spans.push(Span::raw(format!(" @{}", theme::hms_utc(d.at_ms))));
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
        format!(" @{}", theme::hms_utc(d.delivered_at_ms)),
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
