//! Addresses view: a Miller-column drill-down — address directory (left) → that
//! address's recent messages (middle) → detail (right).

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::app::{AddrFocus, AppState};
use crate::data::AddressEntry;
use crate::ui::{detail, theme};
use telex::model::InboxItem;

pub fn render(f: &mut Frame, area: Rect, st: &AppState) {
    let cols = Layout::horizontal([
        Constraint::Percentage(30),
        Constraint::Percentage(35),
        Constraint::Percentage(35),
    ])
    .split(area);

    let addrs = st.visible_addresses();
    render_addresses(
        f,
        cols[0],
        &addrs,
        st.addr_sel,
        st.addr_focus == AddrFocus::List,
    );
    render_messages(
        f,
        cols[1],
        &st.addr_msgs,
        st.addr_msg_sel,
        st.addr_focus == AddrFocus::Messages,
    );
    let selected = st.addr_msgs.get(st.addr_msg_sel).map(|i| &i.message);
    detail::render(f, cols[2], st, selected);
}

fn render_addresses(f: &mut Frame, area: Rect, addrs: &[&AddressEntry], sel: usize, focused: bool) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title(" addresses ", focused));
    if addrs.is_empty() {
        f.render_widget(
            Paragraph::new("(no addresses)")
                .style(Style::default().fg(Color::DarkGray))
                .block(block),
            area,
        );
        return;
    }
    let items: Vec<ListItem> = addrs
        .iter()
        .map(|a| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    theme::occ_symbol(a.occupancy).to_string(),
                    Style::default().fg(theme::occ_color(a.occupancy)),
                ),
                Span::raw(" "),
                Span::raw(a.address.address.clone()),
            ]))
        })
        .collect();
    let mut state = ListState::default();
    state.select(Some(sel.min(addrs.len().saturating_sub(1))));
    f.render_stateful_widget(
        List::new(items)
            .block(block)
            .highlight_style(highlight(focused)),
        area,
        &mut state,
    );
}

fn render_messages(f: &mut Frame, area: Rect, msgs: &[InboxItem], sel: usize, focused: bool) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title(" messages ", focused));
    if msgs.is_empty() {
        f.render_widget(
            Paragraph::new("(select an address)")
                .style(Style::default().fg(Color::DarkGray))
                .block(block),
            area,
        );
        return;
    }
    let items: Vec<ListItem> = msgs
        .iter()
        .map(|it| {
            let m = &it.message;
            let subject = m
                .subject
                .clone()
                .unwrap_or_else(|| m.body.lines().next().unwrap_or("").to_string());
            let disp = theme::disp_label(it.latest_disposition.as_deref()).to_string();
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("#{:<4} ", m.id),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    theme::attn_symbol(&m.attention).to_string(),
                    Style::default().fg(theme::attn_color(&m.attention)),
                ),
                Span::raw(format!(" {} ", m.from_addr.as_deref().unwrap_or("?"))),
                Span::raw(subject),
                Span::styled(
                    format!("  ({disp})"),
                    Style::default().fg(theme::disp_color(it.latest_disposition.as_deref())),
                ),
            ]))
        })
        .collect();
    let mut state = ListState::default();
    state.select(Some(sel.min(msgs.len().saturating_sub(1))));
    f.render_stateful_widget(
        List::new(items)
            .block(block)
            .highlight_style(highlight(focused)),
        area,
        &mut state,
    );
}

fn title(text: &str, focused: bool) -> Span<'static> {
    if focused {
        Span::styled(
            text.to_string(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw(text.to_string())
    }
}

fn highlight(focused: bool) -> Style {
    if focused {
        Style::default()
            .bg(Color::Rgb(40, 40, 55))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().bg(Color::Rgb(30, 30, 38))
    }
}
