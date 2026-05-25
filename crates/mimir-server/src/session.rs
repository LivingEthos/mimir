//! Session management for the Mimir JSON-RPC server.
//!
//! Sessions track per-client state including context packets, provider
//! selections, and conversation history.

use dashmap::DashMap;
use mimir_runs::RunId;
use mimir_schemas::ContextPacket;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Unique session identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionId(pub String);

impl SessionId {
    /// Generate a new random session ID.
    pub fn generate() -> Self {
        Self(RunId::generate().to_string())
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Per-session state.
#[derive(Debug, Clone)]
pub struct Session {
    /// Session identifier.
    pub id: SessionId,
    /// When the session was created.
    pub created_at: Instant,
    /// Last activity timestamp.
    pub last_activity: Instant,
    /// Currently selected provider.
    pub provider: Option<String>,
    /// Currently selected model.
    pub model: Option<String>,
    /// Context packet associated with this session (if any).
    pub context_packet: Option<ContextPacket>,
    /// Conversation history (provider-neutral messages as JSON values).
    pub history: Vec<serde_json::Value>,
}

impl Session {
    /// Create a new session.
    pub fn new(id: SessionId) -> Self {
        let now = Instant::now();
        Self {
            id,
            created_at: now,
            last_activity: now,
            provider: None,
            model: None,
            context_packet: None,
            history: Vec::new(),
        }
    }

    /// Mark the session as recently active.
    pub fn touch(&mut self) {
        self.last_activity = Instant::now();
    }

    /// Check if the session has been idle longer than the given duration.
    pub fn is_idle(&self, duration: Duration) -> bool {
        self.last_activity.elapsed() > duration
    }
}

/// Thread-safe session store.
#[derive(Debug, Clone, Default)]
pub struct SessionStore {
    inner: Arc<DashMap<SessionId, Session>>,
}

impl SessionStore {
    /// Create a new empty session store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new session and return its ID.
    pub fn create(&self) -> SessionId {
        let id = SessionId::generate();
        let session = Session::new(id.clone());
        self.inner.insert(id.clone(), session);
        id
    }

    /// Get a clone of a session by ID.
    pub fn get(&self, id: &SessionId) -> Option<Session> {
        self.inner.get(id).map(|entry| entry.clone())
    }

    /// Update a session (replaces the entry).
    pub fn update(&self, session: Session) {
        self.inner.insert(session.id.clone(), session);
    }

    /// Remove a session by ID.
    pub fn remove(&self, id: &SessionId) -> Option<Session> {
        self.inner.remove(id).map(|(_, s)| s)
    }

    /// List all session IDs.
    pub fn list_ids(&self) -> Vec<SessionId> {
        self.inner.iter().map(|entry| entry.key().clone()).collect()
    }

    /// Remove sessions idle longer than the given duration.
    pub fn purge_idle(&self, duration: Duration) -> usize {
        let to_remove: Vec<SessionId> = self
            .inner
            .iter()
            .filter(|entry| entry.value().is_idle(duration))
            .map(|entry| entry.key().clone())
            .collect();
        let count = to_remove.len();
        for id in to_remove {
            self.inner.remove(&id);
        }
        count
    }

    /// Number of active sessions.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_create_and_get() {
        let store = SessionStore::new();
        let id = store.create();
        assert!(!id.0.is_empty());
        let session = store.get(&id).unwrap();
        assert_eq!(session.id.0, id.0);
    }

    #[test]
    fn session_update_and_remove() {
        let store = SessionStore::new();
        let id = store.create();
        let mut session = store.get(&id).unwrap();
        session.provider = Some("anthropic".to_string());
        session.model = Some("claude-sonnet-4-20250514".to_string());
        store.update(session);
        let updated = store.get(&id).unwrap();
        assert_eq!(updated.provider, Some("anthropic".to_string()));
        assert_eq!(updated.model, Some("claude-sonnet-4-20250514".to_string()));
        store.remove(&id);
        assert!(store.get(&id).is_none());
    }

    #[test]
    fn session_purge_idle() {
        let store = SessionStore::new();
        let id = store.create();
        // Immediately purge with zero duration should remove it.
        let removed = store.purge_idle(Duration::from_secs(0));
        assert_eq!(removed, 1);
        assert!(store.get(&id).is_none());
    }

    #[test]
    fn session_list_ids() {
        let store = SessionStore::new();
        let id1 = store.create();
        let id2 = store.create();
        let ids = store.list_ids();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&id1));
        assert!(ids.contains(&id2));
    }
}
