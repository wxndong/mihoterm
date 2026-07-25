use crate::{
    mihomo::{ApiError, OperatingMode},
    probe::ProbeTarget,
};

use super::{PolicyGroup, ProxyRow, Snapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Groups,
    Proxies,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Search,
    Confirm,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    None,
    Refresh,
    Execute(Operation),
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    SelectProxy { group: String, proxy: String },
    SetMode { mode: OperatingMode },
    Probe { proxy: String, target: ProbeTarget },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationSuccess {
    ProxySelected {
        group: String,
        proxy: String,
    },
    ModeChanged {
        mode: OperatingMode,
    },
    ProbeMeasured {
        proxy: String,
        target: String,
        delay_ms: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PendingChange {
    SelectProxy { group: String, proxy: String },
    SetMode { mode: OperatingMode },
}

#[derive(Debug)]
pub struct App {
    pub controller: String,
    pub snapshot: Option<Snapshot>,
    pub focus: Focus,
    pub input_mode: InputMode,
    pub search: String,
    pub status: StatusLine,
    probes: Vec<ProbeTarget>,
    probe_index: usize,
    pending: Option<PendingChange>,
    operation_in_flight: bool,
    selected_group: usize,
    selected_proxy: usize,
}

impl App {
    #[must_use]
    pub fn new(controller: String) -> Self {
        Self::with_probes(controller, ProbeTarget::built_in())
    }

    #[must_use]
    pub fn with_probes(controller: String, mut probes: Vec<ProbeTarget>) -> Self {
        if probes.is_empty() {
            probes = ProbeTarget::built_in();
        }

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
            probes,
            probe_index: 0,
            pending: None,
            operation_in_flight: false,
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
                if self.status.kind != StatusKind::Ready {
                    self.status = StatusLine {
                        kind: StatusKind::Ready,
                        message: "Connected".into(),
                    };
                }
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

    pub fn apply_operation_result(&mut self, result: Result<OperationSuccess, ApiError>) {
        self.operation_in_flight = false;
        match result {
            Ok(OperationSuccess::ProxySelected { group, proxy }) => {
                if let Some(group_state) = self
                    .snapshot
                    .as_mut()
                    .and_then(|snapshot| snapshot.groups.iter_mut().find(|item| item.name == group))
                {
                    group_state.selected = Some(proxy.clone());
                }
                self.status = StatusLine {
                    kind: StatusKind::Ready,
                    message: format!("Selected {proxy} in {group}"),
                };
            }
            Ok(OperationSuccess::ModeChanged { mode }) => {
                if let Some(snapshot) = &mut self.snapshot {
                    snapshot.mode = mode.to_string();
                }
                self.status = StatusLine {
                    kind: StatusKind::Ready,
                    message: format!("Mode changed to {mode}"),
                };
            }
            Ok(OperationSuccess::ProbeMeasured {
                proxy,
                target,
                delay_ms,
            }) => {
                if let Some(proxy_state) = self.snapshot.as_mut().and_then(|snapshot| {
                    snapshot
                        .groups
                        .iter_mut()
                        .flat_map(|group| group.proxies.iter_mut())
                        .find(|item| item.name == proxy)
                }) {
                    proxy_state.delay_ms = Some(delay_ms);
                    proxy_state.alive = Some(true);
                }
                self.status = StatusLine {
                    kind: StatusKind::Ready,
                    message: format!("{target}: {proxy} responded in {delay_ms} ms"),
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

    pub fn reject_operation(&mut self) {
        self.operation_in_flight = false;
        self.status = StatusLine {
            kind: StatusKind::Error,
            message: "Another operation is already queued".into(),
        };
    }

    #[must_use]
    pub fn current_probe(&self) -> &ProbeTarget {
        &self.probes[self.probe_index]
    }

    pub fn handle_input(&mut self, input: Input) -> Action {
        if input == Input::Quit {
            return Action::Quit;
        }

        if self.input_mode == InputMode::Search {
            return self.handle_search_input(input);
        }
        if self.input_mode == InputMode::Confirm {
            return self.handle_confirm_input(input);
        }

        match input {
            Input::Up => self.move_selection(-1),
            Input::Down => self.move_selection(1),
            Input::Home => self.move_to_edge(false),
            Input::End => self.move_to_edge(true),
            Input::Left => self.focus = Focus::Groups,
            Input::Right
                if self
                    .selected_group()
                    .is_some_and(|group| !group.proxies.is_empty()) =>
            {
                self.focus = Focus::Proxies;
            }
            Input::Enter => self.begin_proxy_selection(),
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
            Input::Character('m') | Input::Character('M') => self.begin_mode_change(),
            Input::Character('p') | Input::Character('P') => self.cycle_probe(),
            Input::Character('d') | Input::Character('D') => {
                if let Some(operation) = self.begin_probe() {
                    return Action::Execute(operation);
                }
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

    fn handle_confirm_input(&mut self, input: Input) -> Action {
        match input {
            Input::Enter | Input::Character('y' | 'Y') => {
                let Some(pending) = self.pending.take() else {
                    self.input_mode = InputMode::Normal;
                    return Action::None;
                };
                self.input_mode = InputMode::Normal;
                self.operation_in_flight = true;
                let operation = match pending {
                    PendingChange::SelectProxy { group, proxy } => {
                        self.status = StatusLine {
                            kind: StatusKind::Info,
                            message: format!("Selecting {proxy}..."),
                        };
                        Operation::SelectProxy { group, proxy }
                    }
                    PendingChange::SetMode { mode } => {
                        self.status = StatusLine {
                            kind: StatusKind::Info,
                            message: format!("Changing mode to {mode}..."),
                        };
                        Operation::SetMode { mode }
                    }
                };
                Action::Execute(operation)
            }
            Input::Escape | Input::Character('n' | 'N' | 'q' | 'Q') => {
                self.pending = None;
                self.input_mode = InputMode::Normal;
                self.status = StatusLine {
                    kind: StatusKind::Info,
                    message: "Change cancelled".into(),
                };
                Action::None
            }
            _ => Action::None,
        }
    }

    fn begin_proxy_selection(&mut self) {
        if self.focus == Focus::Groups {
            if self
                .selected_group()
                .is_some_and(|group| !group.proxies.is_empty())
            {
                self.focus = Focus::Proxies;
            }
            return;
        }
        if self.operation_in_flight {
            self.report_busy();
            return;
        }

        let Some(group) = self.selected_group() else {
            return;
        };
        let Some(proxy) = group.proxies.get(self.selected_proxy) else {
            return;
        };
        if group.selected.as_deref() == Some(proxy.name.as_str()) {
            self.status = StatusLine {
                kind: StatusKind::Info,
                message: format!("{} is already selected", proxy.name),
            };
            return;
        }

        let group_name = group.name.clone();
        let proxy_name = proxy.name.clone();
        self.pending = Some(PendingChange::SelectProxy {
            group: group_name.clone(),
            proxy: proxy_name.clone(),
        });
        self.input_mode = InputMode::Confirm;
        self.status = StatusLine {
            kind: StatusKind::Info,
            message: format!("Select {proxy_name} in {group_name}?"),
        };
    }

    fn begin_mode_change(&mut self) {
        if self.operation_in_flight {
            self.report_busy();
            return;
        }
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let mode = OperatingMode::from_api(&snapshot.mode)
            .unwrap_or(OperatingMode::Direct)
            .next();
        self.pending = Some(PendingChange::SetMode { mode });
        self.input_mode = InputMode::Confirm;
        self.status = StatusLine {
            kind: StatusKind::Info,
            message: format!("Change mode to {mode}?"),
        };
    }

    fn begin_probe(&mut self) -> Option<Operation> {
        if self.operation_in_flight {
            self.report_busy();
            return None;
        }
        let proxy = self.selected_proxy()?.name.clone();
        let target = self.current_probe().clone();
        self.operation_in_flight = true;
        self.status = StatusLine {
            kind: StatusKind::Info,
            message: format!("Probing {proxy} via {}...", target.name()),
        };
        Some(Operation::Probe { proxy, target })
    }

    fn cycle_probe(&mut self) {
        self.probe_index = (self.probe_index + 1) % self.probes.len();
        self.status = StatusLine {
            kind: StatusKind::Info,
            message: format!("Probe target: {}", self.current_probe().name()),
        };
    }

    fn report_busy(&mut self) {
        self.status = StatusLine {
            kind: StatusKind::Info,
            message: "Wait for the current operation to finish".into(),
        };
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
    use super::{
        Action, App, Focus, Input, InputMode, Operation, OperationSuccess, PolicyGroup, ProxyRow,
        Snapshot,
    };
    use crate::mihomo::OperatingMode;

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

    #[test]
    fn proxy_selection_requires_confirmation() {
        let mut app = loaded_app();
        app.handle_input(Input::Right);
        app.handle_input(Input::Up);

        assert_eq!(app.handle_input(Input::Enter), Action::None);
        assert_eq!(app.input_mode, InputMode::Confirm);
        assert_eq!(
            app.handle_input(Input::Character('y')),
            Action::Execute(Operation::SelectProxy {
                group: "Auto".into(),
                proxy: "Proxy A".into(),
            })
        );
    }

    #[test]
    fn mode_change_requires_confirmation() {
        let mut app = loaded_app();

        app.handle_input(Input::Character('m'));

        assert_eq!(app.input_mode, InputMode::Confirm);
        assert_eq!(
            app.handle_input(Input::Enter),
            Action::Execute(Operation::SetMode {
                mode: OperatingMode::Global,
            })
        );
    }

    #[test]
    fn probe_uses_the_selected_target_without_confirmation() {
        let mut app = loaded_app();
        app.handle_input(Input::Right);
        app.handle_input(Input::Character('p'));

        let action = app.handle_input(Input::Character('d'));

        assert!(matches!(
            action,
            Action::Execute(Operation::Probe { proxy, target })
                if proxy == "Proxy B" && target.name() == "OpenAI / Codex"
        ));
    }

    #[test]
    fn operation_result_updates_visible_state() {
        let mut app = loaded_app();

        app.apply_operation_result(Ok(OperationSuccess::ProbeMeasured {
            proxy: "Proxy B".into(),
            target: "GitHub".into(),
            delay_ms: 55,
        }));
        assert_eq!(
            app.selected_proxy().and_then(|proxy| proxy.delay_ms),
            Some(55)
        );
        app.apply_refresh(Ok(snapshot()));

        assert_eq!(app.status.message, "GitHub: Proxy B responded in 55 ms");
    }
}
