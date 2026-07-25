use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, ListState, Paragraph},
};

use crate::app::{App, Focus, InputMode, StatusKind};

const MIN_WIDTH: u16 = 52;
const MIN_HEIGHT: u16 = 10;

pub fn render(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        frame.render_widget(
            Paragraph::new("MihoTerm needs a terminal of at least 52x10.")
                .block(Block::bordered().title(" MihoTerm ")),
            area,
        );
        return;
    }

    let vertical = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(4),
        Constraint::Length(3),
    ])
    .split(area);
    render_header(frame, vertical[0], app);

    let horizontal = Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(vertical[1]);
    render_groups(frame, horizontal[0], app);
    render_proxies(frame, horizontal[1], app);
    render_footer(frame, vertical[2], app);
}

fn render_header(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let (version, mode) = app.snapshot.as_ref().map_or(("...", "..."), |snapshot| {
        (snapshot.version.as_str(), snapshot.mode.as_str())
    });
    let line = Line::from(vec![
        Span::styled(
            "MihoTerm",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(
            "  Mihomo {version}  mode {mode}  {}",
            app.controller
        )),
    ]);
    frame.render_widget(
        Paragraph::new(line).block(Block::bordered().title(" Status ")),
        area,
    );
}

fn render_groups(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let indices = app.visible_group_indices();
    let items = app
        .snapshot
        .as_ref()
        .map(|snapshot| {
            indices
                .iter()
                .map(|index| {
                    let group = &snapshot.groups[*index];
                    let selected = group.selected.as_deref().unwrap_or("-");
                    ListItem::new(format!("{}  ->  {selected}", group.name))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut state = ListState::default().with_selected(app.selected_group_position());
    let block = pane_block(" Policy groups ", app.focus == Focus::Groups);
    let list = List::new(items)
        .block(block)
        .highlight_symbol("> ")
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );

    frame.render_stateful_widget(list, area, &mut state);
}

fn render_proxies(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let indices = app.visible_proxy_indices();
    let items = app
        .selected_group()
        .map(|group| {
            indices
                .iter()
                .map(|index| {
                    let proxy = &group.proxies[*index];
                    let current = if group.selected.as_deref() == Some(proxy.name.as_str()) {
                        "*"
                    } else {
                        " "
                    };
                    let health = match proxy.alive {
                        Some(true) => "+",
                        Some(false) => "x",
                        None => "?",
                    };
                    let delay = proxy
                        .delay_ms
                        .map_or_else(|| "--".into(), |delay| format!("{delay} ms"));
                    ListItem::new(format!("{current} {health}  {}  [{delay}]", proxy.name))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut state = ListState::default().with_selected(app.selected_proxy_position());
    let title = app
        .selected_group()
        .map_or_else(|| " Proxies ".into(), |group| format!(" {} ", group.name));
    let block = pane_block(&title, app.focus == Focus::Proxies);
    let list = List::new(items)
        .block(block)
        .highlight_symbol("> ")
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );

    frame.render_stateful_widget(list, area, &mut state);
}

fn render_footer(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let status_style = match app.status.kind {
        StatusKind::Info => Style::default().fg(Color::Yellow),
        StatusKind::Ready => Style::default().fg(Color::Green),
        StatusKind::Error => Style::default().fg(Color::Red),
    };
    let help = match app.input_mode {
        InputMode::Search => format!("/{}  Enter keep  Esc clear", app.search),
        InputMode::Normal => "Arrows move/focus  Tab switch  / search  r refresh  q quit".into(),
    };
    let line = Line::from(vec![
        Span::styled(&app.status.message, status_style),
        Span::raw("  |  "),
        Span::raw(help),
    ]);

    frame.render_widget(
        Paragraph::new(line).block(Block::bordered().title(" Controls ")),
        area,
    );
}

fn pane_block<'a>(title: &'a str, focused: bool) -> Block<'a> {
    let style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    Block::bordered().title(title).border_style(style)
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::render;
    use crate::app::{App, PolicyGroup, ProxyRow, Snapshot};

    #[test]
    fn renders_a_read_only_snapshot() {
        let mut app = App::new("http://127.0.0.1:9090".into());
        app.apply_refresh(Ok(Snapshot {
            version: "v1.19.29".into(),
            mode: "rule".into(),
            groups: vec![PolicyGroup {
                name: "Auto".into(),
                kind: "URLTest".into(),
                selected: Some("Proxy A".into()),
                proxies: vec![ProxyRow {
                    name: "Proxy A".into(),
                    kind: "Shadowsocks".into(),
                    alive: Some(true),
                    delay_ms: Some(31),
                }],
            }],
        }));
        let backend = TestBackend::new(90, 20);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");

        terminal
            .draw(|frame| render(frame, &app))
            .expect("snapshot should render");
        let rendered = terminal.backend().buffer().content().to_vec();
        let text = rendered
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(text.contains("MihoTerm"));
        assert!(text.contains("Auto"));
        assert!(text.contains("Proxy A"));
        assert!(text.contains("31 ms"));
    }
}
