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

    let footer_lines = footer_lines(app, area.width.saturating_sub(2));
    let desired_footer_height = u16::try_from(footer_lines.len())
        .unwrap_or(u16::MAX)
        .saturating_add(2);
    let footer_height = desired_footer_height.min(area.height.saturating_sub(1));
    let remaining_height = area.height.saturating_sub(footer_height);
    let header_height = if remaining_height >= 7 {
        3
    } else if remaining_height >= 2 {
        1
    } else {
        0
    };
    let vertical = Layout::vertical([
        Constraint::Length(header_height),
        Constraint::Min(1),
        Constraint::Length(footer_height),
    ])
    .split(area);
    if header_height > 0 {
        render_header(frame, vertical[0], app);
    }

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
    render_footer(frame, vertical[2], footer_lines);
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
    let paragraph = Paragraph::new(line);
    if area.height >= 3 {
        frame.render_widget(paragraph.block(Block::bordered().title(" Status ")), area);
    } else {
        frame.render_widget(paragraph, area);
    }
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

fn render_footer(frame: &mut Frame<'_>, area: Rect, lines: Vec<Line<'static>>) {
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(" Controls ")),
        area,
    );
}

fn footer_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    let status_style = match app.status.kind {
        StatusKind::Info => Style::default().fg(Color::Yellow),
        StatusKind::Ready => Style::default().fg(Color::Green),
        StatusKind::Error => Style::default().fg(Color::Red),
    };
    let controls = match app.input_mode {
        InputMode::Search => vec![
            format!("/{}", app.search),
            "Enter keep".into(),
            "Esc clear".into(),
        ],
        InputMode::Confirm => vec!["y/Enter confirm".into(), "n/Esc cancel".into()],
        InputMode::ProfileId => vec![
            format!("Profile ID: {}", app.profile_id_input()),
            "Enter next".into(),
            "Ctrl-U clear".into(),
            "Esc cancel".into(),
        ],
        InputMode::SubscriptionUrl => vec![
            format!(
                "Subscription URL: hidden ({} chars)",
                app.subscription_input_len()
            ),
            "Enter save".into(),
            "Ctrl-U clear".into(),
            "Esc cancel".into(),
        ],
        InputMode::Normal if app.page == Page::Profiles => vec![
            "Up/Down choose".into(),
            "a add".into(),
            "e replace URL".into(),
            "u update".into(),
            "s/Esc back".into(),
            "q quit".into(),
        ],
        InputMode::Normal => vec![
            "Arrows move/focus".into(),
            "Enter select".into(),
            "m mode".into(),
            format!("d probe [{}]", app.current_probe().name()),
            "p target".into(),
            "/ search".into(),
            "r refresh".into(),
            "s sources".into(),
            "q quit".into(),
        ],
    };

    let mut lines = wrap_text(&app.status.message, width)
        .into_iter()
        .map(|line| Line::styled(line, status_style))
        .collect::<Vec<_>>();
    lines.extend(wrap_chunks(&controls, width).into_iter().map(Line::from));
    lines
}

fn wrap_chunks(chunks: &[String], width: u16) -> Vec<String> {
    let width = usize::from(width.max(1));
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_width = 0;

    for chunk in chunks.iter().filter(|chunk| !chunk.is_empty()) {
        let chunk_width = display_width(chunk);
        if chunk_width > width {
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
                current_width = 0;
            }
            let mut wrapped = wrap_text(chunk, u16::try_from(width).unwrap_or(u16::MAX));
            if let Some(last) = wrapped.pop() {
                lines.extend(wrapped);
                current_width = display_width(&last);
                current = last;
            }
            continue;
        }

        let separator_width = usize::from(!current.is_empty()) * 2;
        if current_width + separator_width + chunk_width <= width {
            if !current.is_empty() {
                current.push_str("  ");
            }
            current.push_str(chunk);
            current_width += separator_width + chunk_width;
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(chunk);
            current_width = chunk_width;
        }
    }

    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

fn wrap_text(text: &str, width: u16) -> Vec<String> {
    let width = usize::from(width.max(1));
    let mut lines = Vec::new();

    for logical_line in text.split('\n') {
        let mut current = String::new();
        let mut current_width = 0;

        for word in logical_line.split_whitespace() {
            let word_width = display_width(word);
            let separator_width = usize::from(!current.is_empty());
            if word_width <= width && current_width + separator_width + word_width <= width {
                if !current.is_empty() {
                    current.push(' ');
                }
                current.push_str(word);
                current_width += separator_width + word_width;
                continue;
            }

            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
                current_width = 0;
            }
            if word_width <= width {
                current.push_str(word);
                current_width = word_width;
                continue;
            }

            for character in word.chars() {
                let character_width = display_width(&character.to_string());
                if current_width + character_width > width && !current.is_empty() {
                    lines.push(std::mem::take(&mut current));
                    current_width = 0;
                }
                current.push(character);
                current_width += character_width;
            }
        }

        if !current.is_empty() {
            lines.push(current);
        } else if logical_line.is_empty() {
            lines.push(String::new());
        }
    }

    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn display_width(text: &str) -> usize {
    Line::from(text).width()
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

    use super::{MIN_HEIGHT, MIN_WIDTH, render};
    use crate::{
        app::{App, Input, PolicyGroup, ProxyRow, Snapshot, StatusKind},
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

    #[test]
    fn minimum_terminal_wraps_status_and_keeps_every_control_visible() {
        let mut app = App::new("http://127.0.0.1:9090".into());
        app.status.kind = StatusKind::Ready;
        app.status.message =
            "GitHub: a deliberately long proxy name responded successfully in 55 ms".into();
        let backend = TestBackend::new(MIN_WIDTH, MIN_HEIGHT);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");

        terminal
            .draw(|frame| render(frame, &app))
            .expect("minimum terminal should render");
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(text.contains("GitHub:"));
        assert!(text.contains("55 ms"));
        assert!(text.contains("Arrows move/focus"));
        assert!(text.contains("d probe [Google]"));
        assert!(text.contains("s sources"));
        assert!(text.contains("q quit"));
    }

    #[test]
    fn minimum_terminal_keeps_hidden_subscription_prompt_controls_visible() {
        let mut app = App::with_managed_profiles(
            "managed".into(),
            Vec::new(),
            "default".into(),
            vec![ProfileSummary {
                id: "default".into(),
                has_backup: false,
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
        let backend = TestBackend::new(MIN_WIDTH, MIN_HEIGHT);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");

        terminal
            .draw(|frame| render(frame, &app))
            .expect("minimum terminal should render");
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(text.contains("hidden ("));
        assert!(text.contains("Enter save"));
        assert!(text.contains("Ctrl-U clear"));
        assert!(text.contains("Esc cancel"));
        assert!(!text.contains("private"));
        assert!(!text.contains("never-render"));
    }
}
