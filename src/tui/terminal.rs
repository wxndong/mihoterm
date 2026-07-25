use ratatui::DefaultTerminal;

use crate::app::App;

use super::view;

pub struct TerminalSession {
    terminal: DefaultTerminal,
}

impl TerminalSession {
    pub fn enter() -> std::io::Result<Self> {
        ratatui::try_init().map(|terminal| Self { terminal })
    }

    pub fn draw(&mut self, app: &App) -> std::io::Result<()> {
        self.terminal.draw(|frame| view::render(frame, app))?;
        Ok(())
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = ratatui::try_restore();
    }
}
