//! Input event source. `crossterm::event::read()` is blocking, so it runs on a
//! dedicated OS thread and forwards events into an unbounded channel the async loop
//! selects over alongside its poll timers. No blocking happens on the async runtime.

use ratatui::crossterm::event::{self, Event};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

/// Spawn the input reader thread and return the receiving end of its event channel.
/// The thread exits when the receiver is dropped or the terminal closes.
pub fn input_events() -> UnboundedReceiver<Event> {
    let (tx, rx) = unbounded_channel();
    std::thread::spawn(move || {
        while let Ok(ev) = event::read() {
            if tx.send(ev).is_err() {
                break;
            }
        }
    });
    rx
}
