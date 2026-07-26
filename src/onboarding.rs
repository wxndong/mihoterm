use std::io::{self, IsTerminal, Write};

use crossterm::{
    event::{
        DisableBracketedPaste, EnableBracketedPaste, Event, EventStream, KeyCode, KeyEvent,
        KeyEventKind, KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode},
};
use futures_util::StreamExt;
use secrecy::SecretString;
use thiserror::Error;

use crate::profile::{ProfileError, ProfileSource, ProfileStore, ProfileSummary};

const DEFAULT_PROFILE: &str = "default";
const MAX_SECRET_BYTES: usize = 16 * 1024;

#[derive(Debug, Error)]
pub enum OnboardingError {
    #[error(transparent)]
    Profile(#[from] ProfileError),

    #[error("first-run setup requires an interactive terminal")]
    TerminalRequired,

    #[error("failed to read the subscription URL from the terminal")]
    TerminalInput,

    #[error("first-run setup was cancelled")]
    Cancelled,

    #[error("the subscription URL input exceeds 16 KiB")]
    InputTooLarge,

    #[error("multiple profiles exist; specify one with `mihoterm run PROFILE`")]
    ProfileRequired,
}

pub async fn resolve_profile(
    store: &ProfileStore,
    requested: Option<&str>,
) -> Result<String, OnboardingError> {
    if let Some(profile) = requested {
        return Ok(profile.to_owned());
    }

    let profiles = store.list()?;
    if let Some(profile) = select_existing_profile(&profiles)? {
        return Ok(profile);
    }

    eprintln!("First-run setup");
    eprintln!("Paste your HTTPS subscription URL below. Input is hidden.");
    let source = ProfileSource::from_url(prompt_hidden("Subscription URL: ").await?)?;
    eprintln!("Downloading and validating the profile...");
    store.add(DEFAULT_PROFILE, source).await?;
    eprintln!("Saved as profile '{DEFAULT_PROFILE}'. Starting MihoTerm.");
    Ok(DEFAULT_PROFILE.to_owned())
}

fn select_existing_profile(profiles: &[ProfileSummary]) -> Result<Option<String>, OnboardingError> {
    if profiles.is_empty() {
        return Ok(None);
    }
    if profiles.iter().any(|profile| profile.id == DEFAULT_PROFILE) {
        return Ok(Some(DEFAULT_PROFILE.to_owned()));
    }
    if profiles.len() == 1 {
        return Ok(Some(profiles[0].id.clone()));
    }
    Err(OnboardingError::ProfileRequired)
}

async fn prompt_hidden(prompt: &str) -> Result<SecretString, OnboardingError> {
    if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        return Err(OnboardingError::TerminalRequired);
    }

    let _session = PromptSession::enter(prompt)?;
    let mut events = EventStream::new();
    let mut value = String::new();

    loop {
        let event = events
            .next()
            .await
            .ok_or(OnboardingError::TerminalInput)?
            .map_err(|_| OnboardingError::TerminalInput)?;
        match event {
            Event::Key(key) if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
                match handle_key(key, &mut value)? {
                    InputAction::Continue => {}
                    InputAction::Submit if !value.is_empty() => {
                        return Ok(SecretString::from(value));
                    }
                    InputAction::Submit => {}
                    InputAction::Cancel => return Err(OnboardingError::Cancelled),
                }
            }
            Event::Paste(pasted) => {
                push_input(&mut value, pasted.trim_end_matches(['\r', '\n']))?;
            }
            _ => {}
        }
    }
}

fn handle_key(key: KeyEvent, value: &mut String) -> Result<InputAction, OnboardingError> {
    match (key.code, key.modifiers) {
        (KeyCode::Enter, _) => Ok(InputAction::Submit),
        (KeyCode::Esc, _) => Ok(InputAction::Cancel),
        (KeyCode::Char('c' | 'd'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
            Ok(InputAction::Cancel)
        }
        (KeyCode::Char('u'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
            value.clear();
            Ok(InputAction::Continue)
        }
        (KeyCode::Backspace, _) => {
            value.pop();
            Ok(InputAction::Continue)
        }
        (KeyCode::Char(character), modifiers) if !modifiers.contains(KeyModifiers::CONTROL) => {
            push_input(value, &character.to_string())?;
            Ok(InputAction::Continue)
        }
        _ => Ok(InputAction::Continue),
    }
}

fn push_input(value: &mut String, input: &str) -> Result<(), OnboardingError> {
    if input.contains(['\r', '\n']) {
        return Err(OnboardingError::TerminalInput);
    }
    if value.len().saturating_add(input.len()) > MAX_SECRET_BYTES {
        return Err(OnboardingError::InputTooLarge);
    }
    value.push_str(input);
    Ok(())
}

enum InputAction {
    Continue,
    Submit,
    Cancel,
}

struct PromptSession;

impl PromptSession {
    fn enter(prompt: &str) -> Result<Self, OnboardingError> {
        let mut output = io::stderr();
        output
            .write_all(prompt.as_bytes())
            .and_then(|()| output.flush())
            .map_err(|_| OnboardingError::TerminalInput)?;
        enable_raw_mode().map_err(|_| OnboardingError::TerminalInput)?;
        if execute!(output, EnableBracketedPaste).is_err() {
            let _ = disable_raw_mode();
            return Err(OnboardingError::TerminalInput);
        }
        Ok(Self)
    }
}

impl Drop for PromptSession {
    fn drop(&mut self) {
        let mut output = io::stderr();
        let _ = execute!(output, DisableBracketedPaste);
        let _ = disable_raw_mode();
        let _ = writeln!(output);
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_PROFILE, OnboardingError, ProfileSummary, select_existing_profile};

    #[test]
    fn first_run_requests_setup() {
        assert_eq!(
            select_existing_profile(&[]).expect("empty state should be valid"),
            None
        );
    }

    #[test]
    fn default_profile_wins_when_multiple_profiles_exist() {
        let profiles = vec![profile("alpha"), profile(DEFAULT_PROFILE), profile("zeta")];

        assert_eq!(
            select_existing_profile(&profiles).expect("default should be selected"),
            Some(DEFAULT_PROFILE.to_owned())
        );
    }

    #[test]
    fn one_non_default_profile_is_unambiguous() {
        assert_eq!(
            select_existing_profile(&[profile("primary")]).expect("one profile should work"),
            Some("primary".to_owned())
        );
    }

    #[test]
    fn multiple_non_default_profiles_require_a_choice() {
        let error = select_existing_profile(&[profile("alpha"), profile("zeta")])
            .expect_err("multiple profiles should be ambiguous");

        assert!(matches!(error, OnboardingError::ProfileRequired));
    }

    fn profile(id: &str) -> ProfileSummary {
        ProfileSummary {
            id: id.to_owned(),
            has_backup: false,
        }
    }
}
