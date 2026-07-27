use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, ListState, Paragraph},
};

use crate::app::{App, Focus, InputMode, Page, StatusKind};

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

    match app.page {
        Page::Dashboard => {
            let horizontal =
                Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)])
                    .split(vertical[1]);
            render_groups(frame, horizontal[0], app);
            render_proxies(frame, horizontal[1], app);
        }
        Page::Profiles => render_profiles(frame, vertical[1], app),
    }
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

fn render_profiles(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let horizontal =
        Layout::horizontal([Constraint::Percentage(38), Constraint::Percentage(62)]).split(area);
    let items = app
        .profiles()
        .iter()
        .map(|profile| {
            let active = if app.active_profile() == Some(profile.id.as_str()) {
                "*"
            } else {
                " "
            };
            ListItem::new(format!(
                "{active} {}  [{}]",
                profile.id, profile.source.kind
            ))
        })
        .collect::<Vec<_>>();
    let mut state = ListState::default().with_selected(app.selected_profile_position());
    let list = List::new(items)
        .block(pane_block(" Profiles ", true))
        .highlight_symbol("> ")
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_stateful_widget(list, horizontal[0], &mut state);

    let details = app.selected_profile().map_or_else(
        || "No managed profiles.".to_owned(),
        |profile| {
            let active = if app.active_profile() == Some(profile.id.as_str()) {
                "yes"
            } else {
                "no"
            };
            let backup = if profile.has_backup {
                "available"
            } else {
                "none"
            };
            format!(
                "Profile: {}\nActive: {active}\nSource type: {}\nSource: {}\nPrevious version: {backup}\n\nThe source address is deliberately masked. URL paths and query credentials are never drawn on screen.",
                profile.id, profile.source.kind, profile.source.display
            )
        },
    );
    frame.render_widget(
        Paragraph::new(details)
            .wrap(ratatui::widgets::Wrap { trim: true })
            .block(pane_block(" Subscription source ", false)),
        horizontal[1],
    );
}

fn render_footer(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let status_style = match app.status.kind {
        StatusKind::Info => Style::default().fg(Color::Yellow),
        StatusKind::Ready => Style::default().fg(Color::Green),
        StatusKind::Error => Style::default().fg(Color::Red),
    };
    let help = match app.input_mode {
        InputMode::Search => format!("/{}  Enter keep  Esc clear", app.search),
        InputMode::Confirm => "y/Enter confirm  n/Esc cancel".into(),
        InputMode::ProfileId => format!(
            "Profile ID: {}  Enter next  Ctrl-U clear  Esc cancel",
            app.profile_id_input()
        ),
        InputMode::SubscriptionUrl => format!(
            "Subscription URL: hidden ({} chars)  Enter save  Ctrl-U clear  Esc cancel",
            app.subscription_input_len()
        ),
        InputMode::Normal if app.page == Page::Profiles => {
            "Up/Down choose  a add  e replace URL  u update  s/Esc back  q quit".into()
        }
        InputMode::Normal => format!(
            "Arrows move/focus  Enter select  m mode  d probe [{}]  p target  / search  r refresh  s sources  q quit",
            app.current_probe().name()
        ),
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
    use crate::{
        app::{App, Input, PolicyGroup, ProxyRow, Snapshot},
        profile::{ProfileSourceSummary, ProfileSummary},
    };

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

    #[test]
    fn renders_profile_management_without_exposing_url_credentials() {
        let mut app = App::with_managed_profiles(
            "managed".into(),
            Vec::new(),
            "default".into(),
            vec![ProfileSummary {
                id: "default".into(),
                has_backup: true,
                source: ProfileSourceSummary {
                    kind: "https",
                    display: "https://example.com/…".into(),
                },
            }],
        );
        app.handle_input(Input::Character('s'));
        app.handle_input(Input::Character('e'));
        app.handle_input(Input::Paste(
            "https://example.com/private?token=never-render".into(),
        ));
        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");

        terminal
            .draw(|frame| render(frame, &app))
            .expect("profile page should render");
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(text.contains("https://example.com/"));
        assert!(text.contains("hidden ("));
        assert!(!text.contains("private"));
        assert!(!text.contains("never-render"));
    }
}
