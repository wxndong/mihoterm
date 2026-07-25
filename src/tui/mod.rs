mod terminal;
mod view;

use std::time::Duration;

use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures_util::StreamExt;
use thiserror::Error;
use tokio::{
    signal::unix::{SignalKind, signal},
    sync::mpsc,
};

use crate::{
    app::{Action, App, Input, InputMode, Operation, OperationSuccess, Snapshot, fetch_snapshot},
    mihomo::{ApiClient, ApiError},
};

use terminal::TerminalSession;

#[derive(Debug, Error)]
pub enum TuiError {
    #[error("terminal I/O failed")]
    Terminal(#[from] std::io::Error),

    #[error("terminal input stream ended")]
    InputEnded,
}

pub async fn run(
    client: ApiClient,
    mut app: App,
    refresh_interval: Duration,
) -> Result<(), TuiError> {
    let mut terminal = TerminalSession::enter()?;
    let mut events = EventStream::new();
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut terminate = signal(SignalKind::terminate())?;
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
                event = events.next() => {
                    let Some(event) = event else {
                        return Err(TuiError::InputEnded);
                    };
                    let event = event?;
                    if let Event::Key(key) = event
                        && matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat)
                        && let Some(input) = map_key(key, app.input_mode)
                    {
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
                        }
                    }
                }
                _ = interrupt.recv() => break,
                _ = terminate.recv() => break,
            }
        }

        Ok(())
    }
    .await;

    refresh_worker.abort();
    operation_worker.abort();
    result
}

async fn refresh_worker(
    client: ApiClient,
    refresh_interval: Duration,
    snapshot_sender: mpsc::Sender<Result<Snapshot, ApiError>>,
    mut refresh_receiver: mpsc::Receiver<()>,
) {
    loop {
        let snapshot = fetch_snapshot(&client).await;
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

fn map_key(key: KeyEvent, input_mode: InputMode) -> Option<Input> {
    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c' | 'C'))
    {
        return Some(Input::Quit);
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
    }

    #[test]
    fn control_c_always_quits() {
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);

        assert_eq!(map_key(key, InputMode::Search), Some(Input::Quit));
    }
}
