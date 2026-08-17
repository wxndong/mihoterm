use secrecy::{ExposeSecret, SecretString};

use crate::{
    mihomo::{ApiError, OperatingMode},
    probe::ProbeTarget,
    profile::{ProfileError, ProfileSource, ProfileSummary},
    runtime::RuntimeError,
};

use super::{ConnectionRow, PolicyGroup, ProxyRow, Snapshot};

const MAX_SOURCE_INPUT_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Groups,
    Proxies,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Dashboard,
    Profiles,
    Connections,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Search,
    Confirm,
    Mode,
    ProfileId,
    SubscriptionUrl,
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

#[derive(Debug, Clone, PartialEq, Eq)]
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
    Paste(String),
    Clear,
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    None,
    Refresh,
    Execute(Operation),
    ManageProfile(ProfileOperation),
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
pub enum ProfileOperation {
    Add { id: String, source: ProfileSource },
    ReplaceSource { id: String, source: ProfileSource },
    Update { id: String },
    Switch { id: String },
}

#[derive(Debug, thiserror::Error)]
pub enum ProfileOperationError {
    #[error(transparent)]
    Profile(#[from] ProfileError),
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileOperationSuccess {
    Added {
        id: String,
        profiles: Vec<ProfileSummary>,
    },
    SourceReplaced {
        id: String,
        profiles: Vec<ProfileSummary>,
    },
    Updated {
        id: String,
        profiles: Vec<ProfileSummary>,
    },
    Switched {
        id: String,
        profiles: Vec<ProfileSummary>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PendingChange {
    SelectProxy { group: String, proxy: String },
    SetMode { mode: OperatingMode },
    SwitchProfile { id: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProfileDraftKind {
    Add,
    Replace,
}

#[derive(Debug)]
struct ProfileDraft {
    kind: ProfileDraftKind,
    id: String,
}

#[derive(Debug)]
pub struct App {
    pub controller: String,
    pub snapshot: Option<Snapshot>,
    pub page: Page,
    pub focus: Focus,
    pub input_mode: InputMode,
    mode_choice: OperatingMode,
    pub search: String,
    pub status: StatusLine,
    probes: Vec<ProbeTarget>,
    probe_index: usize,
    pending: Option<PendingChange>,
    operation_in_flight: bool,
    selected_group: usize,
    selected_proxy: usize,
    profile_management: bool,
    active_profile: Option<String>,
    profiles: Vec<ProfileSummary>,
    selected_profile: usize,
    profile_id_input: String,
    subscription_input: SecretString,
    profile_draft: Option<ProfileDraft>,
    selected_connection: usize,
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
            page: Page::Dashboard,
            focus: Focus::Groups,
            input_mode: InputMode::Normal,
            mode_choice: OperatingMode::Global,
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
            profile_management: false,
            active_profile: None,
            profiles: Vec::new(),
            selected_profile: 0,
            profile_id_input: String::new(),
            subscription_input: SecretString::from(String::new()),
            profile_draft: None,
            selected_connection: 0,
        }
    }

    #[must_use]
    pub fn with_managed_profiles(
        controller: String,
        probes: Vec<ProbeTarget>,
        active_profile: String,
        profiles: Vec<ProfileSummary>,
    ) -> Self {
        let mut app = Self::with_probes(controller, probes);
        app.profile_management = true;
        app.selected_profile = profiles
            .iter()
            .position(|profile| profile.id == active_profile)
            .unwrap_or_default();
        app.active_profile = Some(active_profile);
        app.profiles = profiles;
        app
    }

    pub fn apply_refresh(&mut self, result: Result<Snapshot, ApiError>) {
        match result {
            Ok(snapshot) => {
                let group_name = self.selected_group().map(|group| group.name.clone());
                let proxy_name = self.selected_proxy().map(|proxy| proxy.name.clone());
                let connection_id = self
                    .selected_connection()
                    .map(|connection| connection.id.clone());
                self.snapshot = Some(snapshot);
                self.restore_selection(group_name.as_deref(), proxy_name.as_deref());
                self.restore_connection_selection(connection_id.as_deref());
                if self.page == Page::Dashboard
                    && (self.status.message == "Connecting..."
                        || self.status.kind == StatusKind::Error)
                {
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

    pub fn apply_profile_operation_result(
        &mut self,
        result: Result<ProfileOperationSuccess, ProfileOperationError>,
    ) -> bool {
        self.operation_in_flight = false;
        match result {
            Ok(ProfileOperationSuccess::Added { id, profiles }) => {
                self.replace_profiles(profiles, &id);
                self.status = StatusLine {
                    kind: StatusKind::Ready,
                    message: format!("Added {id}; restart with that profile to use it"),
                };
            }
            Ok(ProfileOperationSuccess::SourceReplaced { id, profiles }) => {
                self.replace_profiles(profiles, &id);
                self.status = StatusLine {
                    kind: StatusKind::Ready,
                    message: self.profile_change_message("Replaced source for", &id),
                };
            }
            Ok(ProfileOperationSuccess::Updated { id, profiles }) => {
                self.replace_profiles(profiles, &id);
                self.status = StatusLine {
                    kind: StatusKind::Ready,
                    message: self.profile_change_message("Updated", &id),
                };
            }
            Ok(ProfileOperationSuccess::Switched { id, profiles }) => {
                self.active_profile = Some(id.clone());
                self.replace_profiles(profiles, &id);
                self.page = Page::Dashboard;
                self.focus = Focus::Groups;
                self.search.clear();
                self.status = StatusLine {
                    kind: StatusKind::Info,
                    message: format!("Switched to {id}; refreshing dashboard..."),
                };
                return true;
            }
            Err(error) => {
                self.status = StatusLine {
                    kind: StatusKind::Error,
                    message: error.to_string(),
                };
            }
        }
        false
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
                self.search.clear();
                self.focus = Focus::Groups;
                self.ensure_selection_visible();
                if mode == OperatingMode::Global
                    && self
                        .selected_group()
                        .is_some_and(|group| !group.proxies.is_empty())
                {
                    self.focus = Focus::Proxies;
                }
                self.status = StatusLine {
                    kind: StatusKind::Ready,
                    message: format!("模式已切换为 {}", mode.label()),
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

    #[must_use]
    pub const fn mode_choice(&self) -> OperatingMode {
        self.mode_choice
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
        if self.input_mode == InputMode::Mode {
            return self.handle_mode_input(input);
        }
        if self.input_mode == InputMode::ProfileId {
            return self.handle_profile_id_input(input);
        }
        if self.input_mode == InputMode::SubscriptionUrl {
            return self.handle_subscription_input(input);
        }
        if self.page == Page::Profiles {
            return self.handle_profiles_input(input);
        }
        if self.page == Page::Connections {
            return self.handle_connections_input(input);
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
            Input::Character('s') | Input::Character('S') => self.open_profiles(),
            Input::Character('c') | Input::Character('C') => self.open_connections(),
            _ => {}
        }

        Action::None
    }

    #[must_use]
    pub fn profiles(&self) -> &[ProfileSummary] {
        &self.profiles
    }

    #[must_use]
    pub fn selected_profile(&self) -> Option<&ProfileSummary> {
        self.profiles.get(self.selected_profile)
    }

    #[must_use]
    pub fn selected_profile_position(&self) -> Option<usize> {
        (!self.profiles.is_empty()).then_some(self.selected_profile)
    }

    #[must_use]
    pub fn active_profile(&self) -> Option<&str> {
        self.active_profile.as_deref()
    }

    #[must_use]
    pub fn profile_id_input(&self) -> &str {
        &self.profile_id_input
    }

    #[must_use]
    pub fn subscription_input_len(&self) -> usize {
        self.subscription_input.expose_secret().chars().count()
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
        let mode = OperatingMode::from_api(&snapshot.mode);
        let needle = search.to_lowercase();
        snapshot
            .groups
            .iter()
            .enumerate()
            .filter_map(|(index, group)| {
                let reserved = group.name.eq_ignore_ascii_case("GLOBAL")
                    || group.name.eq_ignore_ascii_case("DIRECT");
                let mode_visible = match mode {
                    Some(OperatingMode::Global) => group.name.eq_ignore_ascii_case("GLOBAL"),
                    Some(OperatingMode::Direct) => false,
                    Some(OperatingMode::Rule) => !reserved,
                    None => true,
                };
                let search_visible =
                    needle.is_empty() || group.name.to_lowercase().contains(&needle);
                (mode_visible && search_visible).then_some(index)
            })
            .collect()
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

    #[must_use]
    pub fn selected_connection(&self) -> Option<&ConnectionRow> {
        self.snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.connections.get(self.selected_connection))
    }

    #[must_use]
    pub fn visible_connection_indices(&self) -> Vec<usize> {
        let Some(snapshot) = &self.snapshot else {
            return Vec::new();
        };
        let needle = self.search.to_lowercase();
        snapshot
            .connections
            .iter()
            .enumerate()
            .filter_map(|(index, connection)| {
                let searchable = needle.is_empty()
                    || connection.host.to_lowercase().contains(&needle)
                    || connection.network.to_lowercase().contains(&needle)
                    || connection.rule.to_lowercase().contains(&needle)
                    || connection
                        .chains
                        .iter()
                        .any(|chain| chain.to_lowercase().contains(&needle));
                searchable.then_some(index)
            })
            .collect()
    }

    #[must_use]
    pub fn selected_connection_position(&self) -> Option<usize> {
        self.visible_connection_indices()
            .iter()
            .position(|index| *index == self.selected_connection)
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
            Input::Paste(value) if !value.contains(['\r', '\n']) => {
                self.search.push_str(&value);
                self.ensure_selection_visible();
            }
            Input::Clear => {
                self.search.clear();
                self.ensure_selection_visible();
            }
            _ => {}
        }

        Action::None
    }

    fn handle_profiles_input(&mut self, input: Input) -> Action {
        match input {
            Input::Up => self.move_profile_selection(-1),
            Input::Down => self.move_profile_selection(1),
            Input::Home => self.selected_profile = 0,
            Input::End if !self.profiles.is_empty() => {
                self.selected_profile = self.profiles.len() - 1;
            }
            Input::Escape | Input::Character('s' | 'S') => {
                self.page = Page::Dashboard;
                self.status = StatusLine {
                    kind: StatusKind::Ready,
                    message: "Connected".into(),
                };
            }
            Input::Character('a' | 'A') => self.begin_add_profile(),
            Input::Character('e' | 'E') => self.begin_edit_profile(),
            Input::Character('u' | 'U' | 'r' | 'R') => {
                if let Some(operation) = self.begin_profile_update() {
                    return Action::ManageProfile(operation);
                }
            }
            Input::Enter => self.begin_profile_switch(),
            _ => {}
        }
        Action::None
    }

    fn handle_profile_id_input(&mut self, input: Input) -> Action {
        match input {
            Input::Escape => self.cancel_profile_form(),
            Input::Enter if !self.profile_id_input.is_empty() => {
                if !valid_profile_id(&self.profile_id_input) {
                    self.status = StatusLine {
                        kind: StatusKind::Error,
                        message: "Profile ID must match [A-Za-z0-9][A-Za-z0-9_-]{0,39}".into(),
                    };
                } else {
                    if let Some(draft) = &mut self.profile_draft {
                        draft.id.clone_from(&self.profile_id_input);
                    }
                    self.input_mode = InputMode::SubscriptionUrl;
                    self.status = StatusLine {
                        kind: StatusKind::Info,
                        message: "Paste the HTTPS subscription URL; input stays hidden".into(),
                    };
                }
            }
            Input::Backspace => {
                self.profile_id_input.pop();
            }
            Input::Clear => self.profile_id_input.clear(),
            Input::Character(character) if !character.is_control() => {
                if self.profile_id_input.len() < 40 {
                    self.profile_id_input.push(character);
                }
            }
            Input::Paste(value) if !value.contains(['\r', '\n']) => {
                if value.len() <= 40 {
                    self.profile_id_input.push_str(&value);
                    self.profile_id_input.truncate(40);
                }
            }
            _ => {}
        }
        Action::None
    }

    fn handle_subscription_input(&mut self, input: Input) -> Action {
        match input {
            Input::Escape => self.cancel_profile_form(),
            Input::Enter if !self.subscription_input.expose_secret().is_empty() => {
                return self.submit_profile_form();
            }
            Input::Backspace => {
                let mut value = self.subscription_input.expose_secret().to_owned();
                value.pop();
                self.subscription_input = SecretString::from(value);
            }
            Input::Clear => self.subscription_input = SecretString::from(String::new()),
            Input::Character(character) if !character.is_control() => {
                self.push_subscription_input(&character.to_string());
            }
            Input::Paste(value) => {
                let value = value.trim_end_matches(['\r', '\n']);
                if value.contains(['\r', '\n']) {
                    self.status = StatusLine {
                        kind: StatusKind::Error,
                        message: "Subscription URL input must be one line".into(),
                    };
                } else {
                    self.push_subscription_input(value);
                }
            }
            _ => {}
        }
        Action::None
    }

    fn open_profiles(&mut self) {
        if !self.profile_management {
            self.status = StatusLine {
                kind: StatusKind::Error,
                message: "Profile management is available only for a managed session".into(),
            };
            return;
        }
        self.page = Page::Profiles;
        self.search.clear();
        self.status = StatusLine {
            kind: StatusKind::Info,
            message: "Subscription addresses are masked to protect credentials".into(),
        };
    }

    fn open_connections(&mut self) {
        self.page = Page::Connections;
        self.search.clear();
        self.focus = Focus::Groups;
        self.selected_connection = 0;
        self.status = StatusLine {
            kind: StatusKind::Info,
            message: "Live connections; read-only view".into(),
        };
    }

    fn handle_connections_input(&mut self, input: Input) -> Action {
        match input {
            Input::Up => self.move_connection_selection(-1),
            Input::Down => self.move_connection_selection(1),
            Input::Home => {
                self.selected_connection = self
                    .visible_connection_indices()
                    .first()
                    .copied()
                    .unwrap_or(0);
            }
            Input::End => {
                self.selected_connection = self
                    .visible_connection_indices()
                    .last()
                    .copied()
                    .unwrap_or(0);
            }
            Input::Escape => {
                if self.search.is_empty() {
                    self.page = Page::Dashboard;
                } else {
                    self.search.clear();
                    self.ensure_connection_visible();
                }
            }
            Input::Left => {
                self.search.clear();
                self.page = Page::Dashboard;
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

    fn move_connection_selection(&mut self, offset: isize) {
        let visible = self.visible_connection_indices();
        self.selected_connection = move_in_visible(&visible, self.selected_connection, offset);
    }

    fn ensure_connection_visible(&mut self) {
        let visible = self.visible_connection_indices();
        if !visible.contains(&self.selected_connection) {
            self.selected_connection = visible.first().copied().unwrap_or(0);
        }
    }

    fn restore_connection_selection(&mut self, id: Option<&str>) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        if let Some(id) = id
            && let Some(position) = snapshot.connections.iter().position(|conn| conn.id == id)
        {
            self.selected_connection = position;
            return;
        }
        // The selected connection vanished (or none was tracked): clamp into range.
        self.selected_connection = self
            .selected_connection
            .min(snapshot.connections.len().saturating_sub(1));
    }

    fn begin_add_profile(&mut self) {
        if self.operation_in_flight {
            self.report_busy();
            return;
        }
        self.profile_id_input.clear();
        self.subscription_input = SecretString::from(String::new());
        self.profile_draft = Some(ProfileDraft {
            kind: ProfileDraftKind::Add,
            id: String::new(),
        });
        self.input_mode = InputMode::ProfileId;
        self.status = StatusLine {
            kind: StatusKind::Info,
            message: "Enter a short profile ID".into(),
        };
    }

    fn begin_edit_profile(&mut self) {
        if self.operation_in_flight {
            self.report_busy();
            return;
        }
        let Some(id) = self.selected_profile().map(|profile| profile.id.clone()) else {
            self.status = StatusLine {
                kind: StatusKind::Error,
                message: "No profile is selected".into(),
            };
            return;
        };
        self.subscription_input = SecretString::from(String::new());
        self.profile_draft = Some(ProfileDraft {
            kind: ProfileDraftKind::Replace,
            id: id.clone(),
        });
        self.input_mode = InputMode::SubscriptionUrl;
        self.status = StatusLine {
            kind: StatusKind::Info,
            message: format!("Paste the replacement HTTPS URL for {id}; input stays hidden"),
        };
    }

    fn begin_profile_update(&mut self) -> Option<ProfileOperation> {
        if self.operation_in_flight {
            self.report_busy();
            return None;
        }
        let id = self.selected_profile()?.id.clone();
        self.operation_in_flight = true;
        self.status = StatusLine {
            kind: StatusKind::Info,
            message: format!("Downloading and validating {id}..."),
        };
        Some(ProfileOperation::Update { id })
    }

    fn begin_profile_switch(&mut self) {
        if self.operation_in_flight {
            self.report_busy();
            return;
        }
        let Some(id) = self.selected_profile().map(|profile| profile.id.clone()) else {
            self.status = StatusLine {
                kind: StatusKind::Error,
                message: "No profile is selected".into(),
            };
            return;
        };
        if self.active_profile.as_deref() == Some(id.as_str()) {
            self.status = StatusLine {
                kind: StatusKind::Info,
                message: format!("{id} is already active"),
            };
            return;
        }
        self.pending = Some(PendingChange::SwitchProfile { id: id.clone() });
        self.input_mode = InputMode::Confirm;
        self.status = StatusLine {
            kind: StatusKind::Info,
            message: format!("Switch the managed proxy to {id}?"),
        };
    }

    fn submit_profile_form(&mut self) -> Action {
        let Some(draft) = self.profile_draft.take() else {
            self.input_mode = InputMode::Normal;
            return Action::None;
        };
        let source = match ProfileSource::from_url(self.subscription_input.clone()) {
            Ok(source) => source,
            Err(error) => {
                self.profile_draft = Some(draft);
                self.status = StatusLine {
                    kind: StatusKind::Error,
                    message: error.to_string(),
                };
                return Action::None;
            }
        };
        self.subscription_input = SecretString::from(String::new());
        self.profile_id_input.clear();
        self.input_mode = InputMode::Normal;
        self.operation_in_flight = true;
        self.status = StatusLine {
            kind: StatusKind::Info,
            message: format!("Downloading and validating {}...", draft.id),
        };
        let operation = match draft.kind {
            ProfileDraftKind::Add => ProfileOperation::Add {
                id: draft.id,
                source,
            },
            ProfileDraftKind::Replace => ProfileOperation::ReplaceSource {
                id: draft.id,
                source,
            },
        };
        Action::ManageProfile(operation)
    }

    fn cancel_profile_form(&mut self) {
        self.profile_id_input.clear();
        self.subscription_input = SecretString::from(String::new());
        self.profile_draft = None;
        self.input_mode = InputMode::Normal;
        self.status = StatusLine {
            kind: StatusKind::Info,
            message: "Profile change cancelled".into(),
        };
    }

    fn push_subscription_input(&mut self, value: &str) {
        let current = self.subscription_input.expose_secret();
        if current.len().saturating_add(value.len()) > MAX_SOURCE_INPUT_BYTES {
            self.status = StatusLine {
                kind: StatusKind::Error,
                message: "Subscription URL input exceeds 16 KiB".into(),
            };
            return;
        }
        let mut next = current.to_owned();
        next.push_str(value);
        self.subscription_input = SecretString::from(next);
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
                            message: format!("正在切换到 {}...", mode.label()),
                        };
                        Operation::SetMode { mode }
                    }
                    PendingChange::SwitchProfile { id } => {
                        self.status = StatusLine {
                            kind: StatusKind::Info,
                            message: format!("Switching to {id}..."),
                        };
                        return Action::ManageProfile(ProfileOperation::Switch { id });
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

    fn handle_mode_input(&mut self, input: Input) -> Action {
        match input {
            Input::Up | Input::Left => self.mode_choice = self.mode_choice.previous(),
            Input::Down | Input::Right | Input::Tab => {
                self.mode_choice = self.mode_choice.next();
            }
            Input::Character('g' | 'G') => self.mode_choice = OperatingMode::Global,
            Input::Character('r' | 'R') => self.mode_choice = OperatingMode::Rule,
            Input::Character('d' | 'D') => self.mode_choice = OperatingMode::Direct,
            Input::Enter => {
                let mode = self.mode_choice;
                self.pending = Some(PendingChange::SetMode { mode });
                self.input_mode = InputMode::Confirm;
                self.status = StatusLine {
                    kind: StatusKind::Info,
                    message: format!("确认切换到 {}？", mode.label()),
                };
            }
            Input::Escape | Input::Character('q' | 'Q') => {
                self.input_mode = InputMode::Normal;
                self.status = StatusLine {
                    kind: StatusKind::Info,
                    message: "模式切换已取消".into(),
                };
            }
            _ => {}
        }
        Action::None
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
        self.mode_choice = OperatingMode::from_api(&snapshot.mode).unwrap_or(OperatingMode::Global);
        self.input_mode = InputMode::Mode;
        self.status = StatusLine {
            kind: StatusKind::Info,
            message: "请选择流量模式".into(),
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

    fn replace_profiles(&mut self, profiles: Vec<ProfileSummary>, selected_id: &str) {
        self.selected_profile = profiles
            .iter()
            .position(|profile| profile.id == selected_id)
            .unwrap_or_default();
        self.profiles = profiles;
    }

    fn profile_change_message(&self, verb: &str, id: &str) -> String {
        if self.active_profile.as_deref() == Some(id) {
            format!("{verb} {id}; restart the managed proxy to apply it")
        } else {
            format!("{verb} {id}")
        }
    }

    fn move_profile_selection(&mut self, offset: isize) {
        if self.profiles.is_empty() {
            self.selected_profile = 0;
            return;
        }
        self.selected_profile = (self.selected_profile as isize + offset)
            .rem_euclid(self.profiles.len() as isize) as usize;
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
        if OperatingMode::from_api(
            self.snapshot
                .as_ref()
                .map_or("", |snapshot| snapshot.mode.as_str()),
        ) == Some(OperatingMode::Global)
            && self
                .selected_group()
                .is_some_and(|group| !group.proxies.is_empty())
        {
            self.focus = Focus::Proxies;
        } else if self
            .selected_group()
            .is_none_or(|group| group.proxies.is_empty())
        {
            self.focus = Focus::Groups;
        }
    }
}

fn valid_profile_id(id: &str) -> bool {
    let mut characters = id.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    first.is_ascii_alphanumeric()
        && id.len() <= 40
        && characters
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
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
        Action, App, ConnectionRow, Focus, Input, InputMode, Operation, OperationSuccess, Page,
        PolicyGroup, ProfileOperation, ProfileOperationSuccess, ProxyRow, Snapshot,
    };
    use crate::{
        mihomo::OperatingMode,
        profile::{ProfileSourceSummary, ProfileSummary},
    };

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
            connections: Vec::new(),
            traffic_rate: None,
        }
    }

    fn loaded_app() -> App {
        let mut app = App::new("http://127.0.0.1:9090".into());
        app.apply_refresh(Ok(snapshot()));
        app
    }

    fn managed_app() -> App {
        let mut app = App::with_managed_profiles(
            "managed".into(),
            Vec::new(),
            "default".into(),
            vec![
                ProfileSummary {
                    id: "default".into(),
                    has_backup: false,
                    source: ProfileSourceSummary {
                        kind: "https",
                        display: "https://example.com/…".into(),
                    },
                },
                ProfileSummary {
                    id: "secondary".into(),
                    has_backup: false,
                    source: ProfileSourceSummary {
                        kind: "https",
                        display: "https://example.net/…".into(),
                    },
                },
            ],
        );
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

        assert_eq!(app.input_mode, InputMode::Mode);
        assert_eq!(app.mode_choice(), OperatingMode::Rule);
        app.handle_input(Input::Character('g'));
        assert_eq!(app.handle_input(Input::Enter), Action::None);
        assert_eq!(app.input_mode, InputMode::Confirm);
        assert_eq!(
            app.handle_input(Input::Enter),
            Action::Execute(Operation::SetMode {
                mode: OperatingMode::Global,
            })
        );
    }

    #[test]
    fn global_mode_only_exposes_global_and_focuses_its_nodes() {
        let mut global = snapshot();
        global.mode = "global".into();
        global.groups.push(PolicyGroup {
            name: "GLOBAL".into(),
            kind: "Selector".into(),
            selected: Some("Proxy A".into()),
            proxies: snapshot().groups[0].proxies.clone(),
        });
        let mut app = App::new("http://127.0.0.1:9090".into());

        app.apply_refresh(Ok(global));

        assert_eq!(app.visible_group_indices().len(), 1);
        assert_eq!(
            app.selected_group().map(|group| group.name.as_str()),
            Some("GLOBAL")
        );
        assert_eq!(app.focus, Focus::Proxies);
    }

    #[test]
    fn direct_mode_exposes_no_proxy_selection() {
        let mut direct = snapshot();
        direct.mode = "direct".into();
        let mut app = App::new("http://127.0.0.1:9090".into());

        app.apply_refresh(Ok(direct));

        assert!(app.visible_group_indices().is_empty());
        assert_eq!(app.focus, Focus::Groups);
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

    #[test]
    fn profile_page_shows_masked_sources_and_supports_direction_keys() {
        let mut app = managed_app();

        assert_eq!(app.handle_input(Input::Character('s')), Action::None);

        assert_eq!(app.page, Page::Profiles);
        assert_eq!(
            app.selected_profile()
                .map(|profile| profile.source.display.as_str()),
            Some("https://example.com/…")
        );
        assert_eq!(app.handle_input(Input::Down), Action::None);
        assert_eq!(
            app.selected_profile().map(|profile| profile.id.as_str()),
            Some("secondary")
        );
    }

    #[test]
    fn add_profile_form_keeps_the_url_out_of_debug_output() {
        let mut app = managed_app();
        app.handle_input(Input::Character('s'));
        app.handle_input(Input::Character('a'));
        app.handle_input(Input::Paste("secondary".into()));
        app.handle_input(Input::Enter);
        app.handle_input(Input::Paste(
            "https://example.com/sub?token=do-not-print".into(),
        ));

        let action = app.handle_input(Input::Enter);
        let debug = format!("{action:?}");

        assert!(matches!(
            action,
            Action::ManageProfile(ProfileOperation::Add { id, .. }) if id == "secondary"
        ));
        assert!(!debug.contains("do-not-print"));
        assert_eq!(app.subscription_input_len(), 0);
    }

    #[test]
    fn profile_switch_requires_confirmation_and_refreshes_the_dashboard() {
        let mut app = managed_app();
        app.handle_input(Input::Character('s'));
        app.handle_input(Input::Down);

        assert_eq!(app.handle_input(Input::Enter), Action::None);
        assert_eq!(app.input_mode, InputMode::Confirm);
        assert_eq!(
            app.handle_input(Input::Enter),
            Action::ManageProfile(ProfileOperation::Switch {
                id: "secondary".into(),
            })
        );

        let profiles = app.profiles().to_vec();
        let refresh = app.apply_profile_operation_result(Ok(ProfileOperationSuccess::Switched {
            id: "secondary".into(),
            profiles,
        }));

        assert!(refresh);
        assert_eq!(app.page, Page::Dashboard);
        assert_eq!(app.active_profile(), Some("secondary"));
        assert!(app.status.message.contains("refreshing dashboard"));
    }

    #[test]
    fn editing_the_active_source_reports_that_a_restart_is_required() {
        let mut app = managed_app();
        app.handle_input(Input::Character('s'));
        app.handle_input(Input::Character('e'));
        app.handle_input(Input::Paste("https://example.net/new?token=hidden".into()));

        assert!(matches!(
            app.handle_input(Input::Enter),
            Action::ManageProfile(ProfileOperation::ReplaceSource { id, .. })
                if id == "default"
        ));

        app.apply_profile_operation_result(Ok(ProfileOperationSuccess::SourceReplaced {
            id: "default".into(),
            profiles: vec![ProfileSummary {
                id: "default".into(),
                has_backup: true,
                source: ProfileSourceSummary {
                    kind: "https",
                    display: "https://example.net/…".into(),
                },
            }],
        }));

        assert!(app.status.message.contains("restart"));
        assert!(!app.status.message.contains("hidden"));
    }

    #[test]
    fn c_opens_the_connections_page() {
        let mut app = loaded_app_with_connections();

        assert_eq!(app.page, Page::Dashboard);

        app.handle_input(Input::Character('c'));

        assert_eq!(app.page, Page::Connections);
        assert_eq!(app.selected_connection_position(), Some(0));
    }

    #[test]
    fn connections_search_filters_by_host() {
        let mut app = loaded_app_with_connections();
        app.handle_input(Input::Character('c'));
        assert_eq!(app.visible_connection_indices().len(), 2);

        app.handle_input(Input::Character('/'));
        app.handle_input(Input::Character('b'));

        assert_eq!(app.visible_connection_indices(), vec![1]);
        assert!(app.selected_connection_position().is_none());
    }

    #[test]
    fn connection_selection_survives_refresh_by_id() {
        let mut app = loaded_app_with_connections();
        app.handle_input(Input::Character('c'));
        app.handle_input(Input::Down);
        assert_eq!(
            app.selected_connection().map(|conn| conn.id.as_str()),
            Some("conn-b")
        );

        // The same connection id must remain selected even if the order changes.
        let mut reordered = connections_snapshot();
        reordered.connections.reverse();
        app.apply_refresh(Ok(reordered));

        assert_eq!(
            app.selected_connection().map(|conn| conn.id.as_str()),
            Some("conn-b")
        );
    }

    #[test]
    fn escape_returns_to_the_dashboard_from_connections() {
        let mut app = loaded_app_with_connections();
        app.handle_input(Input::Character('c'));
        app.handle_input(Input::Escape);

        assert_eq!(app.page, Page::Dashboard);
    }

    fn connections_snapshot() -> Snapshot {
        let mut snapshot = snapshot();
        snapshot.connections = vec![
            ConnectionRow {
                id: "conn-a".into(),
                host: "a.example.com".into(),
                network: "tcp".into(),
                chains: vec!["Proxy A".into()],
                rule: "DOMAIN-SUFFIX .com".into(),
                upload: 10,
                download: 20,
            },
            ConnectionRow {
                id: "conn-b".into(),
                host: "b.example.com".into(),
                network: "udp".into(),
                chains: vec!["DIRECT".into()],
                rule: "DIRECT".into(),
                upload: 0,
                download: 0,
            },
        ];
        snapshot
    }

    fn loaded_app_with_connections() -> App {
        let mut app = App::new("http://127.0.0.1:9090".into());
        app.apply_refresh(Ok(connections_snapshot()));
        app
    }
}
