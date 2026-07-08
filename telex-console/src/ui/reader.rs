//! Full-screen, scrollable single-message reader. Opened with `Enter` on a thread node
//! or `o` from any view; the place to read long bodies/metadata in full. Content is
//! pre-wrapped (one rendered row per line) so scroll bounds are exact, with a scrollbar
//! and a `line X/Y` position indicator.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::{
    Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};
use ratatui::Frame;

use crate::app::AppState;
use crate::ui::detail;

pub fn render(f: &mut Frame, area: Rect, st: &AppState) {
    let Some(m) = &st.reader_msg else {
        st.reader_max_scroll.set(0);
        st.reader_page.set(1);
        f.render_widget(
            Paragraph::new("(no message)")
                .style(Style::default().fg(Color::DarkGray))
                .block(Block::default().borders(Borders::ALL).title(" reader ")),
            area,
        );
        return;
    };

    // Inner dimensions (inside the border); reserve one column on the right for the scrollbar.
    let inner_w = area.width.saturating_sub(2);
    let inner_h = area.height.saturating_sub(2);
    let text_w = inner_w.saturating_sub(1).max(1) as usize;

    let (disps, dels) = detail::detail_rows(st, m.id);
    let lines = detail::build_lines(m, disps, dels, Some(text_w));
    let total = lines.len() as u16;

    let max_scroll = total.saturating_sub(inner_h);
    st.reader_max_scroll.set(max_scroll);
    st.reader_page.set(inner_h.max(1));
    let offset = st.reader_scroll.min(max_scroll);

    let pos = if total == 0 { 0 } else { offset + 1 };
    let title = format!(
        " message #{} · {} → {} · [{}] {}  (line {}/{}) ",
        m.id,
        m.from_addr.as_deref().unwrap_or("?"),
        m.to_addr,
        m.attention,
        m.kind,
        pos,
        total
    );

    let block = Block::default().borders(Borders::ALL).title(title);
    f.render_widget(block, area);

    // Text area inside the border, minus the scrollbar column.
    let text_area = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: text_w as u16,
        height: inner_h,
    };
    f.render_widget(Paragraph::new(lines).scroll((offset, 0)), text_area);

    // Scrollbar along the right inner edge.
    let sb_area = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: inner_w,
        height: inner_h,
    };
    let mut sb_state = ScrollbarState::new(max_scroll as usize).position(offset as usize);
    f.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None),
        sb_area,
        &mut sb_state,
    );
}
