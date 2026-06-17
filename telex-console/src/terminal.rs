//! Terminal lifecycle: enter/leave the alternate screen + raw mode, with a panic hook
//! that restores the terminal before the panic is printed (so a crash never leaves the
//! user's shell in raw mode).

use std::io::{self, Stdout};

use anyhow::Result;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::Terminal;

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

/// Enter raw mode + the alternate screen and install the restoring panic hook.
/// Setup is transactional: if any step after `enable_raw_mode` fails, the terminal is
/// restored before returning the error so the shell is never left in raw mode.
pub fn init() -> Result<Tui> {
    install_panic_hook();
    enable_raw_mode()?;
    let mut out = io::stdout();
    if let Err(e) = execute!(out, EnterAlternateScreen) {
        let _ = restore();
        return Err(e.into());
    }
    match Terminal::new(CrosstermBackend::new(out)) {
        Ok(term) => Ok(term),
        Err(e) => {
            let _ = restore();
            Err(e.into())
        }
    }
}

/// Best-effort restore of the terminal to its normal state.
pub fn restore() -> io::Result<()> {
    execute!(io::stdout(), LeaveAlternateScreen)?;
    disable_raw_mode()
}

fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore();
        prev(info);
    }));
}

/// RAII guard that restores the terminal on drop (covers early returns and `?`).
pub struct Guard;

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = restore();
    }
}
