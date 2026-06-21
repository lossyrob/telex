//! Application state, input handling, and the async run loop.
//!
//! Navigation (`on_key`) is pure and UI-free so it can be unit-tested; anything that
//! touches the backend is an async `Cmd` executed by the run loop. Rendering reads state
//! read-only (see [`crate::ui`]).

use std::cell::Cell;
use std::collections::HashMap;
use std::time::Duration;

use anyhow::Result;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind};

use crate::data::{AddressEntry, Store};
use crate::{filter, terminal, ui};
use telex::model::{DeliveryRow, DispositionRow, InboxItem, MessageRow};

/// Maximum messages retained in the in-memory feed ring.
const FEED_CAP: usize = 2000;
/// How often the address directory / occupancy is refreshed (slower than the feed).
const DIRECTORY_TICK_SECS: u64 = 4;

/// Which top-level view is active.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Feed,
    Addresses,
    Thread,
    /// Full-screen, scrollable reader for a single message.
    Reader,
}

/// Within the Addresses view, which column has focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddrFocus {
    List,
    Messages,
}

/// Input mode: normal navigation, or editing the address filter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Filter(String),
}

/// Startup feed backfill policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backfill {
    TailOnly,
    Recent(i64),
    All,
}

impl Backfill {
    pub fn parse(s: &str) -> Result<Self> {
        let t = s.trim();
        if t.eq_ignore_ascii_case("all") {
            return Ok(Backfill::All);
        }
        let n: i64 = t
            .parse()
            .map_err(|_| anyhow::anyhow!("--backfill must be a number, 0, or 'all' (got '{s}')"))?;
        Ok(if n <= 0 {
            Backfill::TailOnly
        } else {
            Backfill::Recent(n)
        })
    }

    /// Initial feed cursor given the current global max id.
    fn initial_cursor(self, max_id: i64) -> i64 {
        match self {
            Backfill::All => 0,
            Backfill::TailOnly => max_id,
            Backfill::Recent(n) => (max_id - n).max(0),
        }
    }
}

/// Async work requested by `on_key` and run by the loop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cmd {
    LoadThread(i64),
    LoadAddressMessages(String),
}

/// All application state.
pub struct AppState {
    pub backend_name: String,
    pub backend_kind: String,
    pub backfill: Backfill,

    pub view: View,
    pub prev_view: View,
    pub should_quit: bool,
    pub tailing: bool,
    pub mode: Mode,
    pub status: Option<String>,

    // Feed
    pub feed: Vec<MessageRow>,
    pub feed_cursor: i64,
    pub feed_sel: usize,
    /// True once the backfill cursor has been seeded from the global max id. Until then
    /// we must not poll, to avoid an unbounded `export` from cursor 0 on a transient error.
    pub feed_seeded: bool,

    // Addresses
    pub addresses: Vec<AddressEntry>,
    pub addr_sel: usize,
    pub addr_focus: AddrFocus,
    pub addr_msgs: Vec<InboxItem>,
    pub addr_msg_sel: usize,
    /// The address `addr_msgs` was loaded for, so we can detect staleness.
    pub addr_msgs_for: Option<String>,

    // Thread
    pub thread: Vec<MessageRow>,
    pub thread_sel: usize,
    pub thread_root: Option<i64>,
    /// Dispositions per message in the open thread (loaded when the thread is opened).
    pub thread_disp: HashMap<i64, Vec<DispositionRow>>,

    // Detail (dispositions for the currently selected message, loaded lazily)
    pub detail_disp: Option<(i64, Vec<DispositionRow>)>,
    /// Delivery records for the currently selected message, loaded lazily.
    pub detail_deliv: Option<(i64, Vec<DeliveryRow>)>,

    // Reader (full-screen single-message view)
    /// The message currently open in the reader (snapshot taken when opened).
    pub reader_msg: Option<MessageRow>,
    /// The view to return to when the reader is closed.
    pub reader_from: View,
    /// Reader vertical scroll offset, in rendered rows.
    pub reader_scroll: u16,
    /// Maximum scroll offset, computed during render and read back when scrolling.
    pub reader_max_scroll: Cell<u16>,
    /// Reader viewport height (page size), computed during render.
    pub reader_page: Cell<u16>,

    pub filter: Option<String>,
}

impl AppState {
    pub fn new(
        backend_name: String,
        backend_kind: String,
        focus_address: Option<String>,
        backfill: Backfill,
    ) -> Self {
        let mut s = Self {
            backend_name,
            backend_kind,
            backfill,
            view: View::Feed,
            prev_view: View::Feed,
            should_quit: false,
            tailing: true,
            mode: Mode::Normal,
            status: None,
            feed: Vec::new(),
            feed_cursor: 0,
            feed_sel: 0,
            feed_seeded: false,
            addresses: Vec::new(),
            addr_sel: 0,
            addr_focus: AddrFocus::List,
            addr_msgs: Vec::new(),
            addr_msg_sel: 0,
            addr_msgs_for: None,
            thread: Vec::new(),
            thread_sel: 0,
            thread_root: None,
            thread_disp: HashMap::new(),
            detail_disp: None,
            detail_deliv: None,
            reader_msg: None,
            reader_from: View::Feed,
            reader_scroll: 0,
            reader_max_scroll: Cell::new(0),
            reader_page: Cell::new(0),
            filter: None,
        };
        if let Some(a) = focus_address {
            if !a.trim().is_empty() {
                s.filter = Some(a);
            }
        }
        s
    }

    // ---- derived views ----

    /// Feed rows passing the active filter, oldest first.
    pub fn visible_feed(&self) -> Vec<&MessageRow> {
        match &self.filter {
            Some(f) => self
                .feed
                .iter()
                .filter(|m| filter::message_matches(f, m))
                .collect(),
            None => self.feed.iter().collect(),
        }
    }

    /// Address entries passing the active filter.
    pub fn visible_addresses(&self) -> Vec<&AddressEntry> {
        match &self.filter {
            Some(f) => self
                .addresses
                .iter()
                .filter(|a| filter::address_matches(f, &a.address.address))
                .collect(),
            None => self.addresses.iter().collect(),
        }
    }

    /// The message id currently selected (depends on view/focus), if any.
    pub fn selected_message_id(&self) -> Option<i64> {
        match self.view {
            View::Feed => self.visible_feed().get(self.feed_sel).map(|m| m.id),
            View::Addresses => self.addr_msgs.get(self.addr_msg_sel).map(|i| i.message.id),
            View::Thread => self.thread.get(self.thread_sel).map(|m| m.id),
            View::Reader => self.reader_msg.as_ref().map(|m| m.id),
        }
    }

    fn selected_feed_thread(&self) -> Option<i64> {
        self.visible_feed().get(self.feed_sel).map(|m| m.thread_id)
    }

    fn selected_addr_thread(&self) -> Option<i64> {
        self.addr_msgs
            .get(self.addr_msg_sel)
            .map(|i| i.message.thread_id)
    }

    fn selected_address(&self) -> Option<String> {
        self.visible_addresses()
            .get(self.addr_sel)
            .map(|a| a.address.address.clone())
    }

    // ---- key handling (pure) ----

    /// Handle a key press. Mutates navigation state and returns async commands to run.
    pub fn on_key(&mut self, key: KeyEvent) -> Vec<Cmd> {
        self.status = None;
        if let Mode::Filter(_) = self.mode {
            return self.on_key_filter(key);
        }
        if self.view == View::Reader {
            return self.on_key_reader(key);
        }
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Tab => return self.cycle_view(),
            KeyCode::Char('t') => {
                self.tailing = !self.tailing;
                if self.tailing {
                    self.feed_sel = self.visible_feed().len().saturating_sub(1);
                }
            }
            KeyCode::Char('f') => self.mode = Mode::Filter(self.filter.clone().unwrap_or_default()),
            KeyCode::Char('o') => self.open_reader(),
            KeyCode::Char('j') | KeyCode::Down => return self.move_down(),
            KeyCode::Char('k') | KeyCode::Up => return self.move_up(),
            KeyCode::Char('g') | KeyCode::Home => self.go_top(),
            KeyCode::Char('G') | KeyCode::End => self.go_bottom(),
            KeyCode::Right | KeyCode::Char('l') => {
                if self.view == View::Addresses {
                    self.addr_focus = AddrFocus::Messages;
                    self.addr_msg_sel = 0;
                }
            }
            KeyCode::Left | KeyCode::Char('h') => {
                if self.view == View::Addresses {
                    self.addr_focus = AddrFocus::List;
                }
            }
            KeyCode::Enter => return self.on_enter(),
            KeyCode::Esc => {
                if self.view == View::Thread {
                    self.view = self.prev_view;
                } else if self.filter.is_some() {
                    self.filter = None;
                    self.clamp_selection();
                    // The selected address may have changed; reconcile its message pane.
                    if self.view == View::Addresses {
                        if let Some(addr) = self.selected_address() {
                            return vec![Cmd::LoadAddressMessages(addr)];
                        }
                    }
                }
            }
            _ => {}
        }
        Vec::new()
    }

    fn on_key_filter(&mut self, key: KeyEvent) -> Vec<Cmd> {
        let Mode::Filter(buf) = &mut self.mode else {
            return Vec::new();
        };
        match key.code {
            KeyCode::Enter => {
                let f = buf.trim().to_string();
                self.filter = if f.is_empty() { None } else { Some(f) };
                self.mode = Mode::Normal;
                self.clamp_selection();
                if self.view == View::Addresses {
                    if let Some(addr) = self.selected_address() {
                        return vec![Cmd::LoadAddressMessages(addr)];
                    }
                }
            }
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Backspace => {
                buf.pop();
            }
            KeyCode::Char(c) => buf.push(c),
            _ => {}
        }
        Vec::new()
    }

    fn cycle_view(&mut self) -> Vec<Cmd> {
        self.view = match self.view {
            View::Feed => View::Addresses,
            View::Addresses => View::Feed,
            // From Thread, Tab returns to where you came from.
            View::Thread => self.prev_view,
            View::Reader => self.reader_from,
        };
        self.enter_addresses_reload()
    }

    /// When the Addresses view becomes active, load messages for the selected address if
    /// the message pane is stale (or unloaded).
    fn enter_addresses_reload(&mut self) -> Vec<Cmd> {
        if self.view == View::Addresses {
            if let Some(addr) = self.selected_address() {
                if self.addr_msgs_for.as_deref() != Some(addr.as_str()) {
                    return vec![Cmd::LoadAddressMessages(addr)];
                }
            }
        }
        Vec::new()
    }

    fn move_down(&mut self) -> Vec<Cmd> {
        match self.view {
            View::Feed => {
                let len = self.visible_feed().len();
                if len > 0 && self.feed_sel + 1 < len {
                    self.feed_sel += 1;
                }
                if self.feed_sel + 1 < len {
                    // moved off the bottom: stop auto-following
                    self.tailing = false;
                }
            }
            View::Addresses => match self.addr_focus {
                AddrFocus::List => {
                    let len = self.visible_addresses().len();
                    if len > 0 && self.addr_sel + 1 < len {
                        self.addr_sel += 1;
                        if let Some(addr) = self.selected_address() {
                            return vec![Cmd::LoadAddressMessages(addr)];
                        }
                    }
                }
                AddrFocus::Messages => {
                    if self.addr_msg_sel + 1 < self.addr_msgs.len() {
                        self.addr_msg_sel += 1;
                    }
                }
            },
            View::Thread => {
                if self.thread_sel + 1 < self.thread.len() {
                    self.thread_sel += 1;
                }
            }
            View::Reader => {}
        }
        Vec::new()
    }

    fn move_up(&mut self) -> Vec<Cmd> {
        match self.view {
            View::Feed => {
                if self.feed_sel > 0 {
                    self.feed_sel -= 1;
                    self.tailing = false;
                }
            }
            View::Addresses => match self.addr_focus {
                AddrFocus::List => {
                    if self.addr_sel > 0 {
                        self.addr_sel -= 1;
                        if let Some(addr) = self.selected_address() {
                            return vec![Cmd::LoadAddressMessages(addr)];
                        }
                    }
                }
                AddrFocus::Messages => {
                    self.addr_msg_sel = self.addr_msg_sel.saturating_sub(1);
                }
            },
            View::Thread => {
                self.thread_sel = self.thread_sel.saturating_sub(1);
            }
            View::Reader => {}
        }
        Vec::new()
    }

    fn go_top(&mut self) {
        match self.view {
            View::Feed => {
                self.feed_sel = 0;
                self.tailing = false;
            }
            View::Addresses => match self.addr_focus {
                AddrFocus::List => self.addr_sel = 0,
                AddrFocus::Messages => self.addr_msg_sel = 0,
            },
            View::Thread => self.thread_sel = 0,
            View::Reader => {}
        }
    }

    fn go_bottom(&mut self) {
        match self.view {
            View::Feed => {
                self.feed_sel = self.visible_feed().len().saturating_sub(1);
                self.tailing = true;
            }
            View::Addresses => match self.addr_focus {
                AddrFocus::List => self.addr_sel = self.visible_addresses().len().saturating_sub(1),
                AddrFocus::Messages => self.addr_msg_sel = self.addr_msgs.len().saturating_sub(1),
            },
            View::Thread => self.thread_sel = self.thread.len().saturating_sub(1),
            View::Reader => {}
        }
    }

    fn on_enter(&mut self) -> Vec<Cmd> {
        match self.view {
            View::Feed => {
                if let Some(tid) = self.selected_feed_thread() {
                    self.prev_view = View::Feed;
                    return vec![Cmd::LoadThread(tid)];
                }
            }
            View::Addresses => {
                if self.addr_focus == AddrFocus::List {
                    self.addr_focus = AddrFocus::Messages;
                    self.addr_msg_sel = 0;
                } else if let Some(tid) = self.selected_addr_thread() {
                    self.prev_view = View::Addresses;
                    return vec![Cmd::LoadThread(tid)];
                }
            }
            // In a thread, Enter opens the selected message in the full-screen reader.
            View::Thread => self.open_reader(),
            View::Reader => {}
        }
        Vec::new()
    }

    /// Open the full-screen reader on the message selected in the current view.
    fn open_reader(&mut self) {
        let msg = match self.view {
            View::Feed => self.visible_feed().get(self.feed_sel).map(|m| (*m).clone()),
            View::Addresses => self
                .addr_msgs
                .get(self.addr_msg_sel)
                .map(|i| i.message.clone()),
            View::Thread => self.thread.get(self.thread_sel).cloned(),
            View::Reader => self.reader_msg.clone(),
        };
        if let Some(m) = msg {
            self.reader_from = self.view;
            self.reader_msg = Some(m);
            self.reader_scroll = 0;
            self.view = View::Reader;
        }
    }

    /// Key handling while the full-screen reader is open: scroll and close.
    fn on_key_reader(&mut self, key: KeyEvent) -> Vec<Cmd> {
        let max = self.reader_max_scroll.get();
        let page = self.reader_page.get().max(1);
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Esc | KeyCode::Left | KeyCode::Char('h') => self.view = self.reader_from,
            KeyCode::Char('j') | KeyCode::Down => {
                self.reader_scroll = (self.reader_scroll + 1).min(max);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.reader_scroll = self.reader_scroll.saturating_sub(1);
            }
            KeyCode::Char(' ') | KeyCode::PageDown => {
                self.reader_scroll = (self.reader_scroll + page).min(max);
            }
            KeyCode::Char('b') | KeyCode::PageUp => {
                self.reader_scroll = self.reader_scroll.saturating_sub(page);
            }
            KeyCode::Char('g') | KeyCode::Home => self.reader_scroll = 0,
            KeyCode::Char('G') | KeyCode::End => self.reader_scroll = max,
            _ => {}
        }
        Vec::new()
    }

    /// Keep selection indices within bounds after data/filter changes.
    fn clamp_selection(&mut self) {
        let fl = self.visible_feed().len();
        if self.feed_sel >= fl {
            self.feed_sel = fl.saturating_sub(1);
        }
        let al = self.visible_addresses().len();
        if self.addr_sel >= al {
            self.addr_sel = al.saturating_sub(1);
        }
        if self.addr_msg_sel >= self.addr_msgs.len() {
            self.addr_msg_sel = self.addr_msgs.len().saturating_sub(1);
        }
    }

    // ---- data application (pure given inputs) ----

    /// Merge newly fetched feed rows (id-ordered) and advance the cursor. Auto-scrolls
    /// to the newest row while tailing.
    pub fn apply_feed(&mut self, rows: Vec<MessageRow>) {
        if rows.is_empty() {
            return;
        }
        if let Some(last) = rows.last() {
            self.feed_cursor = self.feed_cursor.max(last.id);
        }
        self.feed.extend(rows);
        if self.feed.len() > FEED_CAP {
            let drop = self.feed.len() - FEED_CAP;
            self.feed.drain(0..drop);
        }
        if self.tailing {
            self.feed_sel = self.visible_feed().len().saturating_sub(1);
        } else {
            self.clamp_selection();
        }
    }

    // ---- async execution ----

    async fn exec(&mut self, cmd: Cmd, store: &Store) {
        match cmd {
            Cmd::LoadThread(tid) => match store.thread(tid).await {
                Ok(msgs) => {
                    let mut disp = HashMap::new();
                    for m in &msgs {
                        if let Ok(d) = store.dispositions(m.id).await {
                            if !d.is_empty() {
                                disp.insert(m.id, d);
                            }
                        }
                    }
                    self.thread = msgs;
                    self.thread_disp = disp;
                    self.thread_sel = self.thread.len().saturating_sub(1);
                    self.thread_root = Some(tid);
                    self.view = View::Thread;
                }
                Err(e) => self.status = Some(format!("thread load failed: {e}")),
            },
            Cmd::LoadAddressMessages(addr) => match store.address_inbox(&addr, 200).await {
                Ok(items) => {
                    self.addr_msgs = items;
                    self.addr_msg_sel = 0;
                    self.addr_msgs_for = Some(addr);
                }
                Err(e) => self.status = Some(format!("inbox load failed: {e}")),
            },
        }
    }

    /// Seed the feed cursor from the global max id (bounded backfill) and do the first poll.
    async fn init_feed(&mut self, store: &Store) {
        self.seed_cursor(store).await;
        self.poll_feed(store).await;
    }

    async fn seed_cursor(&mut self, store: &Store) {
        match store.max_message_id().await {
            Ok(max) => {
                self.feed_cursor = self.backfill.initial_cursor(max);
                self.feed_seeded = true;
            }
            Err(e) => self.status = Some(format!("backend error: {e}")),
        }
    }

    async fn poll_feed(&mut self, store: &Store) {
        if !self.feed_seeded {
            // Never export from the default cursor 0 (an unbounded full-table load): retry
            // seeding first and skip this tick if the max id still can't be read.
            self.seed_cursor(store).await;
            if !self.feed_seeded {
                return;
            }
        }
        match store.feed_since(self.feed_cursor).await {
            Ok(rows) => self.apply_feed(rows),
            Err(e) => self.status = Some(format!("feed poll failed: {e}")),
        }
    }

    /// Apply a fresh address-directory snapshot (computed off the main loop). Returns a
    /// reload command when the message pane is stale relative to the selected address.
    pub fn apply_addresses(&mut self, addrs: Vec<AddressEntry>) -> Option<Cmd> {
        self.addresses = addrs;
        self.clamp_selection();
        if self.view == View::Addresses && !self.addresses.is_empty() {
            if let Some(addr) = self.selected_address() {
                if self.addr_msgs_for.as_deref() != Some(addr.as_str()) {
                    return Some(Cmd::LoadAddressMessages(addr));
                }
            }
        }
        None
    }

    /// Lazily load dispositions for the currently selected message (only when changed).
    async fn ensure_detail(&mut self, store: &Store) {
        let Some(id) = self.selected_message_id() else {
            return;
        };
        if self.detail_disp.as_ref().map(|(i, _)| *i) == Some(id) {
            return;
        }
        let disps = store.dispositions(id).await.unwrap_or_default();
        self.detail_disp = Some((id, disps));
        let dels = store.deliveries(id).await.unwrap_or_default();
        self.detail_deliv = Some((id, dels));
    }
}

type DirResult = Result<Vec<AddressEntry>, String>;

/// Run the interactive console until the user quits.
pub async fn run(mut state: AppState, store: Store, poll_secs: u64) -> Result<()> {
    let mut term = terminal::init()?;
    let _guard = terminal::Guard;

    let mut input = crate::event::input_events();
    let mut feed_tick = tokio::time::interval(Duration::from_secs(poll_secs.max(1)));
    let mut dir_tick = tokio::time::interval(Duration::from_secs(DIRECTORY_TICK_SECS));

    // The address-directory refresh (an occupancy fan-out across all addresses) runs in a
    // spawned task so it never stalls input or rendering. `dir_inflight` coalesces ticks
    // that arrive while a refresh is still running.
    let (dir_tx, mut dir_rx) = tokio::sync::mpsc::unbounded_channel::<DirResult>();
    let mut dir_inflight = false;

    state.init_feed(&store).await;
    spawn_directory_refresh(&store, &dir_tx, &mut dir_inflight);
    state.ensure_detail(&store).await;

    loop {
        term.draw(|f| ui::render(f, &state))?;
        // Render computed the reader's content height; clamp the scroll offset so a
        // shrunk message (or a resize) can't leave us parked past the end.
        state.reader_scroll = state.reader_scroll.min(state.reader_max_scroll.get());

        tokio::select! {
            maybe = input.recv() => match maybe {
                Some(Event::Key(k)) if k.kind == KeyEventKind::Press => {
                    for cmd in state.on_key(k) {
                        state.exec(cmd, &store).await;
                    }
                }
                Some(_) => {}
                None => state.should_quit = true,
            },
            _ = feed_tick.tick() => state.poll_feed(&store).await,
            _ = dir_tick.tick() => {
                if !dir_inflight {
                    spawn_directory_refresh(&store, &dir_tx, &mut dir_inflight);
                }
            }
            Some(result) = dir_rx.recv() => {
                dir_inflight = false;
                match result {
                    Ok(addrs) => {
                        if let Some(cmd) = state.apply_addresses(addrs) {
                            state.exec(cmd, &store).await;
                        }
                    }
                    Err(e) => state.status = Some(format!("directory refresh failed: {e}")),
                }
            }
        }

        state.ensure_detail(&store).await;
        if state.should_quit {
            break;
        }
    }

    drop(_guard);
    let _ = terminal::restore();
    Ok(())
}

fn spawn_directory_refresh(
    store: &Store,
    tx: &tokio::sync::mpsc::UnboundedSender<DirResult>,
    inflight: &mut bool,
) {
    *inflight = true;
    let store = store.clone();
    let tx = tx.clone();
    tokio::spawn(async move {
        let _ = tx.send(store.addresses().await.map_err(|e| e.to_string()));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(
            KeyCode::Char(c),
            ratatui::crossterm::event::KeyModifiers::NONE,
        )
    }

    fn msg(id: i64, from: &str, to: &str) -> MessageRow {
        MessageRow {
            id,
            thread_id: id,
            parent_id: None,
            from_addr: Some(from.into()),
            to_addr: to.into(),
            cc: None,
            kind: "note".into(),
            attention: "background".into(),
            requires_disposition: false,
            subject: Some(format!("subj {id}")),
            body: "body".into(),
            metadata: None,
            sent_at_ms: 0,
            created_at_ms: 0,
        }
    }

    fn state() -> AppState {
        AppState::new(
            "default".into(),
            "sqlite".into(),
            None,
            Backfill::Recent(200),
        )
    }

    #[test]
    fn backfill_parse() {
        assert_eq!(Backfill::parse("all").unwrap(), Backfill::All);
        assert_eq!(Backfill::parse("0").unwrap(), Backfill::TailOnly);
        assert_eq!(Backfill::parse("50").unwrap(), Backfill::Recent(50));
        assert!(Backfill::parse("nope").is_err());
    }

    #[test]
    fn backfill_initial_cursor() {
        assert_eq!(Backfill::All.initial_cursor(100), 0);
        assert_eq!(Backfill::TailOnly.initial_cursor(100), 100);
        assert_eq!(Backfill::Recent(30).initial_cursor(100), 70);
        assert_eq!(Backfill::Recent(500).initial_cursor(100), 0);
    }

    #[test]
    fn apply_feed_advances_cursor_and_tails() {
        let mut s = state();
        s.apply_feed(vec![msg(1, "a", "b"), msg(2, "a", "b"), msg(3, "a", "b")]);
        assert_eq!(s.feed_cursor, 3);
        assert_eq!(s.feed.len(), 3);
        // tailing => selection at newest
        assert_eq!(s.feed_sel, 2);
    }

    fn addr_entry(a: &str) -> AddressEntry {
        AddressEntry {
            address: telex::model::AddressRow {
                address: a.into(),
                description: None,
                scope: None,
                tags: None,
                status: "active".into(),
                created_at_ms: 0,
            },
            occupancy: crate::data::Occ::Idle,
            undelivered: None,
        }
    }

    #[test]
    fn apply_addresses_reloads_when_stale() {
        let mut s = state();
        s.view = View::Addresses;
        // No messages loaded yet => stale => request load for the selected address.
        let cmd = s.apply_addresses(vec![addr_entry("node:a"), addr_entry("node:b")]);
        assert_eq!(cmd, Some(Cmd::LoadAddressMessages("node:a".into())));
        // Once loaded for node:a, a refresh that keeps node:a selected is not stale.
        s.addr_msgs_for = Some("node:a".into());
        assert_eq!(s.apply_addresses(vec![addr_entry("node:a")]), None);
    }

    #[test]
    fn entering_addresses_requests_message_load() {
        let mut s = state();
        s.addresses = vec![addr_entry("node:a")];
        let cmds = s.on_key(KeyEvent::new(
            KeyCode::Tab,
            ratatui::crossterm::event::KeyModifiers::NONE,
        ));
        assert_eq!(s.view, View::Addresses);
        assert_eq!(cmds, vec![Cmd::LoadAddressMessages("node:a".into())]);
    }

    #[test]
    fn o_opens_reader_and_esc_returns() {
        let mut s = state();
        s.apply_feed(vec![msg(9, "a", "b")]);
        s.on_key(key('o'));
        assert_eq!(s.view, View::Reader);
        assert_eq!(s.reader_from, View::Feed);
        assert_eq!(s.reader_msg.as_ref().map(|m| m.id), Some(9));
        assert_eq!(s.selected_message_id(), Some(9));
        s.on_key(KeyEvent::new(
            KeyCode::Esc,
            ratatui::crossterm::event::KeyModifiers::NONE,
        ));
        assert_eq!(s.view, View::Feed);
    }

    #[test]
    fn enter_in_thread_opens_reader_not_thread_reload() {
        let mut s = state();
        s.view = View::Thread;
        s.prev_view = View::Feed;
        s.thread = vec![msg(3, "a", "b"), msg(4, "a", "b")];
        s.thread_sel = 1;
        let cmds = s.on_key(KeyEvent::new(
            KeyCode::Enter,
            ratatui::crossterm::event::KeyModifiers::NONE,
        ));
        assert!(cmds.is_empty());
        assert_eq!(s.view, View::Reader);
        assert_eq!(s.reader_from, View::Thread);
        assert_eq!(s.reader_msg.as_ref().map(|m| m.id), Some(4));
        // returning from the reader must not clobber the thread's own back-target
        assert_eq!(s.prev_view, View::Feed);
    }

    #[test]
    fn reader_scroll_clamps_to_bounds() {
        let mut s = state();
        s.apply_feed(vec![msg(1, "a", "b")]);
        s.on_key(key('o'));
        s.reader_max_scroll.set(5);
        s.reader_page.set(3);
        // scrolling up at the top stays at 0
        s.on_key(key('k'));
        assert_eq!(s.reader_scroll, 0);
        // line down, then page down clamps at max
        s.on_key(key('j'));
        assert_eq!(s.reader_scroll, 1);
        s.on_key(key(' '));
        assert_eq!(s.reader_scroll, 4);
        s.on_key(KeyEvent::new(
            KeyCode::Char('G'),
            ratatui::crossterm::event::KeyModifiers::NONE,
        ));
        assert_eq!(s.reader_scroll, 5);
        // never exceeds max
        s.on_key(key('j'));
        assert_eq!(s.reader_scroll, 5);
    }

    #[test]
    fn moving_up_stops_tailing() {
        let mut s = state();
        s.apply_feed(vec![msg(1, "a", "b"), msg(2, "a", "b"), msg(3, "a", "b")]);
        assert!(s.tailing);
        s.on_key(key('k'));
        assert!(!s.tailing);
        assert_eq!(s.feed_sel, 1);
        // new data arrives but we should not jump to bottom
        s.apply_feed(vec![msg(4, "a", "b")]);
        assert_eq!(s.feed_sel, 1);
    }

    #[test]
    fn filter_narrows_feed_and_clamps() {
        let mut s = state();
        s.apply_feed(vec![
            msg(1, "impl-215", "orch"),
            msg(2, "node:ci", "orch"),
            msg(3, "impl-215", "orch"),
        ]);
        s.filter = Some("impl".into());
        assert_eq!(s.visible_feed().len(), 2);
        s.clamp_selection();
        assert!(s.feed_sel < 2);
    }

    #[test]
    fn tab_cycles_feed_and_addresses() {
        let mut s = state();
        assert_eq!(s.view, View::Feed);
        s.on_key(KeyEvent::new(
            KeyCode::Tab,
            ratatui::crossterm::event::KeyModifiers::NONE,
        ));
        assert_eq!(s.view, View::Addresses);
        s.on_key(KeyEvent::new(
            KeyCode::Tab,
            ratatui::crossterm::event::KeyModifiers::NONE,
        ));
        assert_eq!(s.view, View::Feed);
    }

    #[test]
    fn enter_on_feed_requests_thread_load() {
        let mut s = state();
        s.apply_feed(vec![msg(7, "a", "b")]);
        let cmds = s.on_key(KeyEvent::new(
            KeyCode::Enter,
            ratatui::crossterm::event::KeyModifiers::NONE,
        ));
        assert_eq!(cmds, vec![Cmd::LoadThread(7)]);
        assert_eq!(s.prev_view, View::Feed);
    }

    #[test]
    fn filter_entry_mode_edits_buffer() {
        let mut s = state();
        s.on_key(key('f'));
        assert!(matches!(s.mode, Mode::Filter(_)));
        s.on_key(key('a'));
        s.on_key(key('b'));
        s.on_key(KeyEvent::new(
            KeyCode::Enter,
            ratatui::crossterm::event::KeyModifiers::NONE,
        ));
        assert_eq!(s.filter.as_deref(), Some("ab"));
        assert_eq!(s.mode, Mode::Normal);
    }
}
