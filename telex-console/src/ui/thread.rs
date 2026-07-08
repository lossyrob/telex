//! Thread view: a threaded transcript (indented by reply depth) with inline disposition
//! summaries, above the shared detail pane for the selected node.

use std::collections::HashMap;

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::app::AppState;
use crate::ui::{detail, theme};
use telex::model::MessageRow;

pub fn render(f: &mut Frame, area: Rect, st: &AppState) {
    let rows = Layout::vertical([Constraint::Min(1), Constraint::Percentage(45)]).split(area);
    render_transcript(f, rows[0], st);
    let selected = st.thread.get(st.thread_sel);
    detail::render(f, rows[1], st, selected);
}

fn render_transcript(f: &mut Frame, area: Rect, st: &AppState) {
    let title = match st.thread_root {
        Some(t) => format!(" thread #{t} ({} msgs) ", st.thread.len()),
        None => " thread ".to_string(),
    };
    let block = Block::default().borders(Borders::ALL).title(title);
    if st.thread.is_empty() {
        f.render_widget(
            Paragraph::new("(empty thread)")
                .style(Style::default().fg(Color::DarkGray))
                .block(block),
            area,
        );
        return;
    }

    let by_id: HashMap<i64, &MessageRow> = st.thread.iter().map(|m| (m.id, m)).collect();
    let items: Vec<ListItem> = st
        .thread
        .iter()
        .map(|m| transcript_item(m, &by_id, st))
        .collect();
    let mut state = ListState::default();
    state.select(Some(st.thread_sel.min(st.thread.len().saturating_sub(1))));
    f.render_stateful_widget(
        List::new(items).block(block).highlight_style(
            Style::default()
                .bg(Color::Rgb(40, 40, 55))
                .add_modifier(Modifier::BOLD),
        ),
        area,
        &mut state,
    );
}

fn transcript_item(
    m: &MessageRow,
    by_id: &HashMap<i64, &MessageRow>,
    st: &AppState,
) -> ListItem<'static> {
    let indent = "  ".repeat(reply_depth(m, by_id));
    let subject = crate::ui::detail::sanitize(
        &m.subject
            .clone()
            .unwrap_or_else(|| m.body.lines().next().unwrap_or("").to_string()),
    );
    let mut line = vec![
        Span::raw(indent),
        Span::styled(
            theme::attn_symbol(&m.attention).to_string(),
            Style::default().fg(theme::attn_color(&m.attention)),
        ),
        Span::styled(format!(" #{} ", m.id), Style::default().fg(Color::DarkGray)),
        Span::raw(format!(
            "{} → {} ",
            m.from_addr.as_deref().unwrap_or("?"),
            m.to_addr
        )),
        Span::styled(format!("[{}] ", m.kind), Style::default().fg(Color::Cyan)),
        Span::raw(subject),
    ];
    if let Some(latest) = st.thread_disp.get(&m.id).and_then(|d| d.last()) {
        line.push(Span::styled(
            format!("  ⤷ {}", latest.state),
            Style::default().fg(theme::disp_color(Some(&latest.state))),
        ));
    }
    ListItem::new(Line::from(line))
}

/// Reply depth = number of ancestors present within this thread (cycle-guarded).
fn reply_depth(m: &MessageRow, by_id: &HashMap<i64, &MessageRow>) -> usize {
    let mut depth = 0;
    let mut cur = m.parent_id;
    let mut guard = 0;
    while let Some(p) = cur {
        if guard > 64 {
            break;
        }
        match by_id.get(&p) {
            Some(pm) => {
                depth += 1;
                cur = pm.parent_id;
            }
            None => break,
        }
        guard += 1;
    }
    depth
}
