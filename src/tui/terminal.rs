use crossterm::{
    event::{DisableBracketedPaste, EnableBracketedPaste},
    execute,
};
use ratatui::DefaultTerminal;

use crate::app::App;

use super::view;

pub struct TerminalSession {
    terminal: DefaultTerminal,
}

impl TerminalSession {
    pub fn enter() -> std::io::Result<Self> {
        let mut terminal = ratatui::try_init()?;
        if let Err(error) = execute!(terminal.backend_mut(), EnableBracketedPaste) {
            let _ = ratatui::try_restore();
            return Err(error);
        }
        Ok(Self { terminal })
    }

    pub fn draw(&mut self, app: &App) -> std::io::Result<()> {
        self.terminal.draw(|frame| view::render(frame, app))?;
        Ok(())
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = execute!(self.terminal.backend_mut(), DisableBracketedPaste);
        let _ = ratatui::try_restore();
    }
}
