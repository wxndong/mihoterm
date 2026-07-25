use crate::mihomo::{ApiClient, ApiError, ProxiesResponse, RuntimeConfig, VersionInfo};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub version: String,
    pub mode: String,
    pub groups: Vec<PolicyGroup>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyGroup {
    pub name: String,
    pub kind: String,
    pub selected: Option<String>,
    pub proxies: Vec<ProxyRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyRow {
    pub name: String,
    pub kind: String,
    pub alive: Option<bool>,
    pub delay_ms: Option<u32>,
}

impl Snapshot {
    fn from_api(version: VersionInfo, config: RuntimeConfig, proxies: ProxiesResponse) -> Self {
        let groups = proxies
            .proxies
            .iter()
            .filter(|(_, proxy)| proxy.is_group())
            .map(|(map_name, group)| {
                let name = if group.name.is_empty() {
                    map_name.clone()
                } else {
                    group.name.clone()
                };
                let members = group
                    .all
                    .iter()
                    .map(|member_name| {
                        let member = proxies.proxies.get(member_name);
                        ProxyRow {
                            name: member_name.clone(),
                            kind: member.map_or_else(String::new, |proxy| proxy.kind.clone()),
                            alive: member.and_then(|proxy| proxy.alive),
                            delay_ms: member.and_then(|proxy| proxy.latest_delay_ms()),
                        }
                    })
                    .collect();

                PolicyGroup {
                    name,
                    kind: group.kind.clone(),
                    selected: group.now.clone(),
                    proxies: members,
                }
            })
            .collect();

        Self {
            version: fallback_text(version.version, "unknown"),
            mode: fallback_text(config.mode.unwrap_or_default(), "unknown"),
            groups,
        }
    }
}

fn fallback_text(value: String, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.into()
    } else {
        value
    }
}

pub async fn fetch_snapshot(client: &ApiClient) -> Result<Snapshot, ApiError> {
    let (version, config, proxies) =
        tokio::join!(client.version(), client.configuration(), client.proxies());

    Ok(Snapshot::from_api(version?, config?, proxies?))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Groups,
    Proxies,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Search,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusKind {
    Info,
    Ready,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusLine {
    pub kind: StatusKind,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    Up,
    Down,
    Left,
    Right,
    Enter,
    Escape,
    Backspace,
    Home,
    End,
    Tab,
    Character(char),
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    Refresh,
    Quit,
}

#[derive(Debug)]
pub struct App {
    pub controller: String,
    pub snapshot: Option<Snapshot>,
    pub focus: Focus,
    pub input_mode: InputMode,
    pub search: String,
    pub status: StatusLine,
    selected_group: usize,
    selected_proxy: usize,
}

impl App {
    #[must_use]
    pub fn new(controller: String) -> Self {
        Self {
            controller,
            snapshot: None,
            focus: Focus::Groups,
            input_mode: InputMode::Normal,
            search: String::new(),
            status: StatusLine {
                kind: StatusKind::Info,
                message: "Connecting...".into(),
            },
            selected_group: 0,
            selected_proxy: 0,
        }
    }

    pub fn apply_refresh(&mut self, result: Result<Snapshot, ApiError>) {
        match result {
            Ok(snapshot) => {
                let group_name = self.selected_group().map(|group| group.name.clone());
                let proxy_name = self.selected_proxy().map(|proxy| proxy.name.clone());
                self.snapshot = Some(snapshot);
                self.restore_selection(group_name.as_deref(), proxy_name.as_deref());
                self.status = StatusLine {
                    kind: StatusKind::Ready,
                    message: "Connected".into(),
                };
            }
            Err(error) => {
                self.status = StatusLine {
                    kind: StatusKind::Error,
                    message: error.to_string(),
                };
            }
        }
    }

    pub fn mark_refreshing(&mut self) {
        self.status = StatusLine {
            kind: StatusKind::Info,
            message: "Refreshing...".into(),
        };
    }

    pub fn handle_input(&mut self, input: Input) -> Action {
        if input == Input::Quit {
            return Action::Quit;
        }

        if self.input_mode == InputMode::Search {
            return self.handle_search_input(input);
        }

        match input {
            Input::Up => self.move_selection(-1),
            Input::Down => self.move_selection(1),
            Input::Home => self.move_to_edge(false),
            Input::End => self.move_to_edge(true),
            Input::Left => self.focus = Focus::Groups,
            Input::Right | Input::Enter
                if self
                    .selected_group()
                    .is_some_and(|group| !group.proxies.is_empty()) =>
            {
                self.focus = Focus::Proxies;
            }
            Input::Tab => {
                self.focus = match self.focus {
                    Focus::Groups
                        if self
                            .selected_group()
                            .is_some_and(|group| !group.proxies.is_empty()) =>
                    {
                        Focus::Proxies
                    }
                    _ => Focus::Groups,
                };
                self.search.clear();
            }
            Input::Escape => {
                if self.search.is_empty() {
                    self.focus = Focus::Groups;
                } else {
                    self.search.clear();
                    self.ensure_selection_visible();
                }
            }
            Input::Character('/') => {
                self.input_mode = InputMode::Search;
            }
            Input::Character('r') | Input::Character('R') => {
                self.mark_refreshing();
                return Action::Refresh;
            }
            _ => {}
        }

        Action::None
    }

    #[must_use]
    pub fn selected_group(&self) -> Option<&PolicyGroup> {
        self.snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.groups.get(self.selected_group))
    }

    #[must_use]
    pub fn selected_proxy(&self) -> Option<&ProxyRow> {
        self.selected_group()
            .and_then(|group| group.proxies.get(self.selected_proxy))
    }

    #[must_use]
    pub fn visible_group_indices(&self) -> Vec<usize> {
        let Some(snapshot) = &self.snapshot else {
            return Vec::new();
        };
        let search = if self.focus == Focus::Groups {
            self.search.as_str()
        } else {
            ""
        };
        visible_indices(
            snapshot.groups.iter().map(|group| group.name.as_str()),
            search,
        )
    }

    #[must_use]
    pub fn visible_proxy_indices(&self) -> Vec<usize> {
        let Some(group) = self.selected_group() else {
            return Vec::new();
        };
        let search = if self.focus == Focus::Proxies {
            self.search.as_str()
        } else {
            ""
        };
        visible_indices(
            group.proxies.iter().map(|proxy| proxy.name.as_str()),
            search,
        )
    }

    #[must_use]
    pub fn selected_group_position(&self) -> Option<usize> {
        self.visible_group_indices()
            .iter()
            .position(|index| *index == self.selected_group)
    }

    #[must_use]
    pub fn selected_proxy_position(&self) -> Option<usize> {
        self.visible_proxy_indices()
            .iter()
            .position(|index| *index == self.selected_proxy)
    }

    fn handle_search_input(&mut self, input: Input) -> Action {
        match input {
            Input::Escape => {
                self.search.clear();
                self.input_mode = InputMode::Normal;
            }
            Input::Enter => self.input_mode = InputMode::Normal,
            Input::Backspace => {
                self.search.pop();
                self.ensure_selection_visible();
            }
            Input::Character(character) if !character.is_control() => {
                self.search.push(character);
                self.ensure_selection_visible();
            }
            _ => {}
        }

        Action::None
    }

    fn move_selection(&mut self, offset: isize) {
        match self.focus {
            Focus::Groups => {
                let visible = self.visible_group_indices();
                self.selected_group = move_in_visible(&visible, self.selected_group, offset);
                self.select_current_proxy();
            }
            Focus::Proxies => {
                let visible = self.visible_proxy_indices();
                self.selected_proxy = move_in_visible(&visible, self.selected_proxy, offset);
            }
        }
    }

    fn move_to_edge(&mut self, end: bool) {
        let visible = match self.focus {
            Focus::Groups => self.visible_group_indices(),
            Focus::Proxies => self.visible_proxy_indices(),
        };
        let Some(index) = (if end { visible.last() } else { visible.first() }) else {
            return;
        };

        match self.focus {
            Focus::Groups => {
                self.selected_group = *index;
                self.select_current_proxy();
            }
            Focus::Proxies => self.selected_proxy = *index,
        }
    }

    fn ensure_selection_visible(&mut self) {
        match self.focus {
            Focus::Groups => {
                let visible = self.visible_group_indices();
                if !visible.contains(&self.selected_group) {
                    self.selected_group = visible.first().copied().unwrap_or_default();
                    self.select_current_proxy();
                }
            }
            Focus::Proxies => {
                let visible = self.visible_proxy_indices();
                if !visible.contains(&self.selected_proxy) {
                    self.selected_proxy = visible.first().copied().unwrap_or_default();
                }
            }
        }
    }

    fn select_current_proxy(&mut self) {
        self.selected_proxy = self
            .selected_group()
            .and_then(|group| {
                let selected = group.selected.as_deref()?;
                group
                    .proxies
                    .iter()
                    .position(|proxy| proxy.name == selected)
            })
            .unwrap_or_default();
    }

    fn restore_selection(&mut self, group_name: Option<&str>, proxy_name: Option<&str>) {
        self.selected_group = group_name
            .and_then(|name| {
                self.snapshot
                    .as_ref()?
                    .groups
                    .iter()
                    .position(|group| group.name == name)
            })
            .unwrap_or_default();
        self.selected_proxy = proxy_name
            .and_then(|name| {
                self.selected_group()?
                    .proxies
                    .iter()
                    .position(|proxy| proxy.name == name)
            })
            .unwrap_or_else(|| {
                self.select_current_proxy();
                self.selected_proxy
            });
        self.ensure_selection_visible();
        if self
            .selected_group()
            .is_none_or(|group| group.proxies.is_empty())
        {
            self.focus = Focus::Groups;
        }
    }
}

fn visible_indices<'a>(names: impl Iterator<Item = &'a str>, search: &str) -> Vec<usize> {
    let needle = search.to_lowercase();
    names
        .enumerate()
        .filter_map(|(index, name)| {
            (needle.is_empty() || name.to_lowercase().contains(&needle)).then_some(index)
        })
        .collect()
}

fn move_in_visible(visible: &[usize], current: usize, offset: isize) -> usize {
    if visible.is_empty() {
        return 0;
    }
    let current_position = visible
        .iter()
        .position(|index| *index == current)
        .unwrap_or_default();
    let next_position =
        (current_position as isize + offset).rem_euclid(visible.len() as isize) as usize;
    visible[next_position]
}

#[cfg(test)]
mod tests {
    use super::{Action, App, Focus, Input, InputMode, PolicyGroup, ProxyRow, Snapshot};

    fn snapshot() -> Snapshot {
        Snapshot {
            version: "v1.19.29".into(),
            mode: "rule".into(),
            groups: vec![
                PolicyGroup {
                    name: "Auto".into(),
                    kind: "URLTest".into(),
                    selected: Some("Proxy B".into()),
                    proxies: vec![
                        ProxyRow {
                            name: "Proxy A".into(),
                            kind: "Shadowsocks".into(),
                            alive: Some(true),
                            delay_ms: Some(30),
                        },
                        ProxyRow {
                            name: "Proxy B".into(),
                            kind: "WireGuard".into(),
                            alive: Some(true),
                            delay_ms: Some(45),
                        },
                    ],
                },
                PolicyGroup {
                    name: "Fallback".into(),
                    kind: "Fallback".into(),
                    selected: None,
                    proxies: Vec::new(),
                },
            ],
        }
    }

    fn loaded_app() -> App {
        let mut app = App::new("http://127.0.0.1:9090".into());
        app.apply_refresh(Ok(snapshot()));
        app
    }

    #[test]
    fn right_arrow_focuses_the_proxy_list_and_selects_current_proxy() {
        let mut app = loaded_app();

        assert_eq!(app.handle_input(Input::Right), Action::None);
        assert_eq!(app.focus, Focus::Proxies);
        assert_eq!(
            app.selected_proxy().map(|proxy| proxy.name.as_str()),
            Some("Proxy B")
        );
    }

    #[test]
    fn directional_navigation_wraps() {
        let mut app = loaded_app();

        app.handle_input(Input::Up);

        assert_eq!(
            app.selected_group().map(|group| group.name.as_str()),
            Some("Fallback")
        );
    }

    #[test]
    fn search_filters_the_focused_list() {
        let mut app = loaded_app();

        app.handle_input(Input::Character('/'));
        app.handle_input(Input::Character('f'));

        assert_eq!(app.input_mode, InputMode::Search);
        assert_eq!(app.visible_group_indices(), vec![1]);
        assert_eq!(
            app.selected_group().map(|group| group.name.as_str()),
            Some("Fallback")
        );
    }

    #[test]
    fn proxy_search_does_not_hide_policy_groups() {
        let mut app = loaded_app();
        app.handle_input(Input::Right);
        app.handle_input(Input::Character('/'));
        app.handle_input(Input::Character('b'));

        assert_eq!(app.visible_group_indices(), vec![0, 1]);
        assert_eq!(app.visible_proxy_indices(), vec![1]);
        assert_eq!(
            app.selected_proxy().map(|proxy| proxy.name.as_str()),
            Some("Proxy B")
        );
    }

    #[test]
    fn refresh_preserves_named_selection() {
        let mut app = loaded_app();
        app.handle_input(Input::Down);
        assert_eq!(
            app.selected_group().map(|group| group.name.as_str()),
            Some("Fallback")
        );

        let mut updated = snapshot();
        updated.groups.reverse();
        app.apply_refresh(Ok(updated));

        assert_eq!(
            app.selected_group().map(|group| group.name.as_str()),
            Some("Fallback")
        );
    }

    #[test]
    fn refresh_key_emits_a_non_blocking_action() {
        let mut app = loaded_app();

        assert_eq!(app.handle_input(Input::Character('r')), Action::Refresh);
        assert_eq!(app.status.message, "Refreshing...");
    }
}
