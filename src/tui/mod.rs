mod terminal;
mod view;

use std::time::{Duration, Instant};

use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures_util::StreamExt;
use thiserror::Error;
use tokio::sync::mpsc;

use crate::{
    app::{
        Action, App, Input, InputMode, Operation, OperationSuccess, ProfileOperation,
        ProfileOperationError, ProfileOperationSuccess, Snapshot, enrich_with_connections,
        fetch_snapshot,
    },
    mihomo::{ApiClient, ApiError},
    profile::ProfileStore,
    runtime::SessionManager,
};

use terminal::TerminalSession;

#[derive(Debug, Error)]
pub enum TuiError {
    #[error("terminal I/O failed")]
    Terminal(#[from] std::io::Error),

    #[error("terminal input stream ended")]
    InputEnded,
}

pub struct ManagedProfiles {
    store: ProfileStore,
    session_manager: SessionManager,
}

impl ManagedProfiles {
    #[must_use]
    pub const fn new(store: ProfileStore, session_manager: SessionManager) -> Self {
        Self {
            store,
            session_manager,
        }
    }
}

pub async fn run(
    client: ApiClient,
    mut app: App,
    refresh_interval: Duration,
    managed_profiles: Option<ManagedProfiles>,
) -> Result<(), TuiError> {
    let mut terminal = TerminalSession::enter()?;
    let mut events = EventStream::new();
    let (snapshot_sender, mut snapshot_receiver) = mpsc::channel(1);
    let (refresh_sender, refresh_receiver) = mpsc::channel(1);
    let refresh_worker = tokio::spawn(refresh_worker(
        client.clone(),
        refresh_interval,
        snapshot_sender,
        refresh_receiver,
    ));
    let (operation_sender, operation_receiver) = mpsc::channel(1);
    let (result_sender, mut result_receiver) = mpsc::channel(1);
    let operation_worker =
        tokio::spawn(operation_worker(client, operation_receiver, result_sender));
    let (profile_sender, profile_receiver) = mpsc::channel(1);
    let (profile_result_sender, mut profile_result_receiver) = mpsc::channel(1);
    let profile_worker = managed_profiles.map(|managed| {
        tokio::spawn(profile_worker(
            managed,
            profile_receiver,
            profile_result_sender,
        ))
    });

    let result = async {
        loop {
            terminal.draw(&app)?;

            tokio::select! {
                snapshot = snapshot_receiver.recv() => {
                    let Some(snapshot) = snapshot else {
                        return Err(TuiError::InputEnded);
                    };
                    app.apply_refresh(snapshot);
                }
                operation = result_receiver.recv() => {
                    let Some(operation) = operation else {
                        return Err(TuiError::InputEnded);
                    };
                    app.apply_operation_result(operation);
                    let _ = refresh_sender.try_send(());
                }
                profile_result = profile_result_receiver.recv(), if profile_worker.is_some() => {
                    let Some(profile_result) = profile_result else {
                        return Err(TuiError::InputEnded);
                    };
                    if app.apply_profile_operation_result(profile_result) {
                        let _ = refresh_sender.try_send(());
                    }
                }
                event = events.next() => {
                    let Some(event) = event else {
                        return Err(TuiError::InputEnded);
                    };
                    let event = event?;
                    let input = match event {
                        Event::Key(key)
                            if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
                        {
                            map_key(key, app.input_mode)
                        }
                        Event::Paste(value) => Some(Input::Paste(value)),
                        _ => None,
                    };
                    if let Some(input) = input {
                        match app.handle_input(input) {
                            Action::None => {}
                            Action::Quit => break,
                            Action::Refresh => {
                                let _ = refresh_sender.try_send(());
                            }
                            Action::Execute(operation) => {
                                if operation_sender.try_send(operation).is_err() {
                                    app.reject_operation();
                                }
                            }
                            Action::ManageProfile(operation) => {
                                if profile_sender.try_send(operation).is_err() {
                                    app.reject_operation();
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }
    .await;

    refresh_worker.abort();
    operation_worker.abort();
    if let Some(worker) = profile_worker {
        worker.abort();
    }
    result
}

async fn refresh_worker(
    client: ApiClient,
    refresh_interval: Duration,
    snapshot_sender: mpsc::Sender<Result<Snapshot, ApiError>>,
    mut refresh_receiver: mpsc::Receiver<()>,
) {
    let mut prev_traffic: Option<(u64, u64, Instant)> = None;
    loop {
        let snapshot = match fetch_snapshot(&client).await {
            Ok(snapshot) => Ok(enrich_with_connections(&client, snapshot, &mut prev_traffic).await),
            Err(error) => {
                prev_traffic = None;
                Err(error)
            }
        };
        if snapshot_sender.send(snapshot).await.is_err() {
            return;
        }

        tokio::select! {
            _ = tokio::time::sleep(refresh_interval) => {}
            refresh = refresh_receiver.recv() => {
                if refresh.is_none() {
                    return;
                }
            }
        }
    }
}

async fn operation_worker(
    client: ApiClient,
    mut operation_receiver: mpsc::Receiver<Operation>,
    result_sender: mpsc::Sender<Result<OperationSuccess, ApiError>>,
) {
    while let Some(operation) = operation_receiver.recv().await {
        let result = match operation {
            Operation::SelectProxy { group, proxy } => client
                .select_proxy(&group, &proxy)
                .await
                .map(|()| OperationSuccess::ProxySelected { group, proxy }),
            Operation::SetMode { mode } => client
                .set_mode(mode)
                .await
                .map(|()| OperationSuccess::ModeChanged { mode }),
            Operation::Probe { proxy, target } => {
                let target_name = target.name().to_owned();
                client.probe_delay(&proxy, &target).await.map(|response| {
                    OperationSuccess::ProbeMeasured {
                        proxy,
                        target: target_name,
                        delay_ms: response.delay,
                    }
                })
            }
        };

        if result_sender.send(result).await.is_err() {
            return;
        }
    }
}

async fn profile_worker(
    managed: ManagedProfiles,
    mut operation_receiver: mpsc::Receiver<ProfileOperation>,
    result_sender: mpsc::Sender<Result<ProfileOperationSuccess, ProfileOperationError>>,
) {
    let ManagedProfiles {
        store,
        session_manager,
    } = managed;
    while let Some(operation) = operation_receiver.recv().await {
        let result = match operation {
            ProfileOperation::Add { id, source } => match store.add(&id, source).await {
                Ok(()) => store
                    .list()
                    .map(|profiles| ProfileOperationSuccess::Added { id, profiles })
                    .map_err(ProfileOperationError::from),
                Err(error) => Err(error.into()),
            },
            ProfileOperation::ReplaceSource { id, source } => {
                match store.replace_source(&id, source).await {
                    Ok(()) => store
                        .list()
                        .map(|profiles| ProfileOperationSuccess::SourceReplaced { id, profiles })
                        .map_err(ProfileOperationError::from),
                    Err(error) => Err(error.into()),
                }
            }
            ProfileOperation::Update { id } => match store.update(&id).await {
                Ok(()) => store
                    .list()
                    .map(|profiles| ProfileOperationSuccess::Updated { id, profiles })
                    .map_err(ProfileOperationError::from),
                Err(error) => Err(error.into()),
            },
            ProfileOperation::Switch { id } => {
                let result = store.profile_path(&id).map_err(ProfileOperationError::from);
                match result {
                    Ok(path) => match session_manager.switch_profile(&id, &path).await {
                        Ok(_) => store
                            .list()
                            .map(|profiles| ProfileOperationSuccess::Switched { id, profiles })
                            .map_err(ProfileOperationError::from),
                        Err(error) => Err(error.into()),
                    },
                    Err(error) => Err(error),
                }
            }
        };

        if result_sender.send(result).await.is_err() {
            return;
        }
    }
}

fn map_key(key: KeyEvent, input_mode: InputMode) -> Option<Input> {
    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c' | 'C'))
    {
        return Some(Input::Quit);
    }
    if key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('u' | 'U'))
        && matches!(
            input_mode,
            InputMode::Search | InputMode::ProfileId | InputMode::SubscriptionUrl
        )
    {
        return Some(Input::Clear);
    }
    if key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return None;
    }

    match key.code {
        KeyCode::Up => Some(Input::Up),
        KeyCode::Down => Some(Input::Down),
        KeyCode::Left => Some(Input::Left),
        KeyCode::Right => Some(Input::Right),
        KeyCode::Enter => Some(Input::Enter),
        KeyCode::Esc => Some(Input::Escape),
        KeyCode::Backspace => Some(Input::Backspace),
        KeyCode::Home => Some(Input::Home),
        KeyCode::End => Some(Input::End),
        KeyCode::Tab => Some(Input::Tab),
        KeyCode::Char('q' | 'Q') if input_mode == InputMode::Normal => Some(Input::Quit),
        KeyCode::Char(character) => Some(Input::Character(character)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::map_key;
    use crate::app::{Input, InputMode};

    #[test]
    fn q_quits_only_in_normal_mode() {
        let key = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);

        assert_eq!(map_key(key, InputMode::Normal), Some(Input::Quit));
        assert_eq!(map_key(key, InputMode::Search), Some(Input::Character('q')));
        assert_eq!(
            map_key(key, InputMode::Confirm),
            Some(Input::Character('q'))
        );
        assert_eq!(map_key(key, InputMode::Mode), Some(Input::Character('q')));
        assert_eq!(
            map_key(key, InputMode::SubscriptionUrl),
            Some(Input::Character('q'))
        );
    }

    #[test]
    fn control_c_always_quits() {
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);

        assert_eq!(map_key(key, InputMode::Search), Some(Input::Quit));
    }
}
