//! Bounded per-scope persistence for Alan interrogation sessions.
//!
//! Stores each [`AlanScope`]'s conversation turns atomically at
//! `<workbench>/alan_sessions/<scope_key>.json`, bounded to the most recent
//! [`DEFAULT_MAX_TURNS`] turns (at most [`MAX_PERSISTED_MESSAGES`] messages),
//! and degrades gracefully on missing or corrupt files.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::alan_interrogator::AlanScope;
use crate::store::{self, WorkspacePaths};

pub const ALAN_SESSION_SCHEMA_VERSION: u32 = 1;
/// Maximum number of conversation turns kept per scope (`30` user+assistant turns).
pub const DEFAULT_MAX_TURNS: usize = 30;
/// Hard ceiling on persisted messages (`DEFAULT_MAX_TURNS * 2`).
pub const MAX_PERSISTED_MESSAGES: usize = DEFAULT_MAX_TURNS * 2;

/// Role of a persisted message in an Alan interrogation session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlanMessageRole {
    User,
    Assistant,
    SystemNote,
}

/// One persisted message inside a [`PersistedAlanSession`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedAlanMessage {
    pub id: String,
    pub role: AlanMessageRole,
    pub text: String,
    #[serde(default)]
    pub raw_text: String,
    #[serde(default)]
    pub created_at_ms: u64,
}

/// Persisted conversation state for a single [`AlanScope`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedAlanSession {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub scope: AlanScope,
    #[serde(default)]
    pub scope_key: String,
    #[serde(default)]
    pub updated_at_ms: u64,
    #[serde(default)]
    pub messages: Vec<PersistedAlanMessage>,
}

const fn default_schema_version() -> u32 {
    ALAN_SESSION_SCHEMA_VERSION
}

impl PersistedAlanSession {
    /// Creates an empty session for `scope`.
    pub fn empty(scope: &AlanScope) -> Self {
        let norm = scope.normalized();
        let scope_key = norm.scope_key();
        Self {
            schema_version: ALAN_SESSION_SCHEMA_VERSION,
            scope: norm,
            scope_key,
            updated_at_ms: 0,
            messages: Vec::new(),
        }
    }

    /// Trims `messages` in-place to at most the last `max_turns` turns
    /// (never exceeding `max_turns * 2` messages).
    pub fn trim_to_max_turns(&mut self, max_turns: usize) {
        if max_turns == 0 {
            self.messages.clear();
            return;
        }
        let max_messages = max_turns.saturating_mul(2);
        let user_indices: Vec<usize> = self
            .messages
            .iter()
            .enumerate()
            .filter_map(|(idx, msg)| (msg.role == AlanMessageRole::User).then_some(idx))
            .collect();

        if user_indices.len() > max_turns {
            let start_idx = user_indices[user_indices.len() - max_turns];
            self.messages.drain(..start_idx);
        }

        if self.messages.len() > max_messages {
            let excess = self.messages.len() - max_messages;
            self.messages.drain(..excess);
            if let Some(first_user) = self
                .messages
                .iter()
                .position(|msg| msg.role == AlanMessageRole::User)
                && first_user > 0
                && first_user < self.messages.len()
            {
                self.messages.drain(..first_user);
            }
        }
    }
}

/// Directory storing persisted Alan sessions (`<workbench>/alan_sessions`).
pub fn alan_sessions_dir(paths: &WorkspacePaths) -> PathBuf {
    paths.workbench_dir.join("alan_sessions")
}

/// Canonical session file path (`<workbench>/alan_sessions/<scope_key>.json`).
pub fn alan_session_path(paths: &WorkspacePaths, scope: &AlanScope) -> PathBuf {
    alan_sessions_dir(paths).join(format!("{}.json", scope.scope_key()))
}

/// Loads a persisted Alan session for `scope`, returning an empty session if
/// the file is missing or corrupt.
pub fn load_alan_session(paths: &WorkspacePaths, scope: &AlanScope) -> PersistedAlanSession {
    let norm = scope.normalized();
    let path = alan_session_path(paths, &norm);
    let bytes = match fs::read(&path) {
        Ok(b) => b,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return PersistedAlanSession::empty(&norm);
        }
        Err(err) => {
            log::warn!(
                "Failed to read Alan session {}: {err}; falling back to empty session",
                path.display()
            );
            return PersistedAlanSession::empty(&norm);
        }
    };

    match serde_json::from_slice::<PersistedAlanSession>(&bytes) {
        Ok(mut session) => {
            session.schema_version = ALAN_SESSION_SCHEMA_VERSION;
            session.scope = norm.clone();
            session.scope_key = norm.scope_key();
            session.trim_to_max_turns(DEFAULT_MAX_TURNS);
            session
        }
        Err(err) => {
            log::warn!(
                "Corrupt Alan session {}: {err}; falling back to empty session",
                path.display()
            );
            PersistedAlanSession::empty(&norm)
        }
    }
}

/// Atomically persists an Alan session (bounded to [`DEFAULT_MAX_TURNS`] turns)
/// to `<workbench>/alan_sessions/<scope_key>.json`.
pub fn save_alan_session(paths: &WorkspacePaths, session: &PersistedAlanSession) -> Result<()> {
    let mut normalized = session.clone();
    normalized.schema_version = ALAN_SESSION_SCHEMA_VERSION;
    normalized.scope = normalized.scope.normalized();
    normalized.scope_key = normalized.scope.scope_key();
    if normalized.updated_at_ms == 0 {
        normalized.updated_at_ms = store::now_ms();
    }
    normalized.trim_to_max_turns(DEFAULT_MAX_TURNS);

    let path = alan_session_path(paths, &normalized.scope);
    store::write_json_atomic(&path, &normalized)
        .with_context(|| format!("保存 Alan 会话失败 {}", path.display()))
}

/// Deletes the persisted Alan session for `scope` if present.
/// Returns `Ok(true)` if a file was removed, or `Ok(false)` if none existed.
pub fn clear_alan_session(paths: &WorkspacePaths, scope: &AlanScope) -> Result<bool> {
    let path = alan_session_path(paths, scope);
    match fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err).with_context(|| format!("清理 Alan 会话失败 {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_workspace(label: &str) -> (PathBuf, WorkspacePaths) {
        let dir = std::env::temp_dir().join(format!(
            "m3-alan-session-{label}-{}-{}",
            std::process::id(),
            store::now_ms()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("workbench")).unwrap();
        let paths = WorkspacePaths::from_root(dir.clone());
        (dir, paths)
    }

    #[test]
    fn session_save_load_clear_roundtrip_and_atomic_cleanup() {
        let (dir, paths) = temp_workspace("roundtrip");
        let scope = AlanScope::Company("cost".to_owned());

        let missing = load_alan_session(&paths, &scope);
        assert!(missing.messages.is_empty());
        assert_eq!(missing.scope, AlanScope::Company("COST".to_owned()));
        assert_eq!(missing.scope_key, "company-COST");

        let mut session = PersistedAlanSession::empty(&scope);
        session.updated_at_ms = 1_700_000_000_000;
        session.messages.push(PersistedAlanMessage {
            id: "u-1".to_owned(),
            role: AlanMessageRole::User,
            text: "Costco 会员续费率是否构筑了护城河？".to_owned(),
            raw_text: "Costco 会员续费率是否构筑了护城河？".to_owned(),
            created_at_ms: 1_700_000_000_001,
        });
        session.messages.push(PersistedAlanMessage {
            id: "a-1".to_owned(),
            role: AlanMessageRole::Assistant,
            text: "续费率长期高于 90% [report:COST]".to_owned(),
            raw_text: "续费率长期高于 90% [report:COST]".to_owned(),
            created_at_ms: 1_700_000_000_002,
        });

        save_alan_session(&paths, &session).unwrap();

        let expected_path = alan_session_path(&paths, &scope);
        assert!(expected_path.ends_with("workbench/alan_sessions/company-COST.json"));
        assert!(expected_path.is_file());

        // Ensure no leftover temporary atomic write files remain.
        let leftover_tmp = fs::read_dir(alan_sessions_dir(&paths))
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().ends_with(".tmp"));
        assert!(!leftover_tmp);

        let loaded = load_alan_session(&paths, &scope);
        assert_eq!(loaded.messages.len(), 2);
        assert_eq!(loaded.messages[0].id, "u-1");
        assert_eq!(loaded.messages[1].role, AlanMessageRole::Assistant);

        assert!(clear_alan_session(&paths, &scope).unwrap());
        assert!(!clear_alan_session(&paths, &scope).unwrap());
        assert!(load_alan_session(&paths, &scope).messages.is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn session_trims_to_last_30_turns_and_handles_corrupt_json_gracefully() {
        let (dir, paths) = temp_workspace("trim-corrupt");
        let scope = AlanScope::Industry("retail".to_owned());

        let mut session = PersistedAlanSession::empty(&scope);
        for turn in 1..=35 {
            session.messages.push(PersistedAlanMessage {
                id: format!("u-{turn}"),
                role: AlanMessageRole::User,
                text: format!("Question {turn}"),
                raw_text: format!("Question {turn}"),
                created_at_ms: turn as u64 * 10,
            });
            session.messages.push(PersistedAlanMessage {
                id: format!("a-{turn}"),
                role: AlanMessageRole::Assistant,
                text: format!("Answer {turn}"),
                raw_text: format!("Answer {turn}"),
                created_at_ms: turn as u64 * 10 + 1,
            });
        }

        save_alan_session(&paths, &session).unwrap();
        let loaded = load_alan_session(&paths, &scope);
        assert_eq!(loaded.messages.len(), DEFAULT_MAX_TURNS * 2);
        assert_eq!(loaded.messages.first().map(|m| m.id.as_str()), Some("u-6"));
        assert_eq!(loaded.messages.last().map(|m| m.id.as_str()), Some("a-35"));

        // Corrupt the session file and confirm graceful empty fallback.
        let path = alan_session_path(&paths, &scope);
        fs::write(&path, b"{not valid json!!!").unwrap();
        let fallback = load_alan_session(&paths, &scope);
        assert!(fallback.messages.is_empty());
        assert_eq!(fallback.scope_key, "industry-retail");

        let _ = fs::remove_dir_all(&dir);
    }
}
