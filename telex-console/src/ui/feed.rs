//! Feed view: the global, live-tailing message stream (left) plus the shared detail
//! pane (right).

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::app::AppState;
use crate::ui::{detail, theme};
use telex::model::MessageRow;

pub fn render(f: &mut Frame, area: Rect, st: &AppState) {
    let cols =
        Layout::horizontal([Constraint::Percentage(60), Constraint::Percentage(40)]).split(area);

    let rows = st.visible_feed();
    render_list(f, cols[0], &rows, st.feed_sel);
    let selected = rows.get(st.feed_sel).copied();
    detail::render(f, cols[1], st, selected);
}

fn render_list(f: &mut Frame, area: Rect, rows: &[&MessageRow], sel: usize) {
    let block = Block::default().borders(Borders::ALL).title(" feed ");
    if rows.is_empty() {
        f.render_widget(
            Paragraph::new("(waiting for messages…)")
                .style(Style::default().fg(Color::DarkGray))
                .block(block),
            area,
        );
        return;
    }

    let items: Vec<ListItem> = rows.iter().map(|m| ListItem::new(row_line(m))).collect();
    let mut state = ListState::default();
    state.select(Some(sel.min(rows.len().saturating_sub(1))));
    let list = List::new(items).block(block).highlight_style(
        Style::default()
            .bg(Color::Rgb(40, 40, 55))
            .add_modifier(Modifier::BOLD),
    );
    f.render_stateful_widget(list, area, &mut state);
}

/// Build a single feed row: `HH:MM:SS A* from → to [kind] subject`.
pub fn row_line(m: &MessageRow) -> Line<'static> {
    let needs = if m.requires_disposition { "*" } else { " " };
    let subject = m
        .subject
        .clone()
        .unwrap_or_else(|| m.body.lines().next().unwrap_or("").to_string());
    Line::from(vec![
        Span::styled(
            theme::hms_utc(m.sent_at_ms),
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw(" "),
        Span::styled(
            theme::attn_symbol(&m.attention).to_string(),
            Style::default().fg(theme::attn_color(&m.attention)),
        ),
        Span::styled(needs.to_string(), Style::default().fg(Color::Red)),
        Span::raw(format!(
            " {} → {} ",
            m.from_addr.as_deref().unwrap_or("?"),
            m.to_addr
        )),
        Span::styled(format!("[{}] ", m.kind), Style::default().fg(Color::Cyan)),
        Span::raw(subject),
    ])
}
