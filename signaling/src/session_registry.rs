use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::redis::SessionState;

#[derive(Clone, Default)]
pub struct SessionRegistry {
    inner: Arc<RwLock<SessionStore>>,
}

type Nicknames = HashSet<String>;

#[derive(Default)]
struct SessionStore {
    by_session_id: HashMap<String, SessionState>,
    by_room_id: HashMap<String, Nicknames>,
}

impl SessionStore {
    fn upsert(&mut self, session_id: String, session: SessionState) {
        if let Some(previous) = self
            .by_session_id
            .insert(session_id.clone(), session.clone())
        {
            if let Some(previous_room) = previous.room_id {
                self.remove_room_link(&session_id, &previous_room);
            }
        }
        if let Some(room_id) = session.room_id {
            self.by_room_id
                .entry(room_id)
                .or_default()
                .insert(session_id);
        }
    }

    fn remove(&mut self, session_id: &str) {
        if let Some(previous) = self.by_session_id.remove(session_id) {
            if let Some(previous_room) = previous.room_id {
                self.remove_room_link(session_id, &previous_room);
            }
        }
    }

    fn set_room(&mut self, session_id: &str, room_id: Option<String>) {
        let previous_room = self
            .by_session_id
            .get(session_id)
            .and_then(|session| session.room_id.clone());
        if !self.by_session_id.contains_key(session_id) {
            return;
        }

        if let Some(previous_room) = previous_room {
            self.remove_room_link(session_id, &previous_room);
        }

        if let Some(new_room) = room_id.clone() {
            self.by_room_id
                .entry(new_room.clone())
                .or_default()
                .insert(session_id.to_owned());
        }
        if let Some(session) = self.by_session_id.get_mut(session_id) {
            session.room_id = room_id;
        }
    }

    fn clear_room(&mut self, room_id: &str) -> Vec<String> {
        let Some(session_ids) = self.by_room_id.remove(room_id) else {
            return Vec::new();
        };
        let mut affected = Vec::with_capacity(session_ids.len());
        for session_id in session_ids {
            if let Some(session) = self.by_session_id.get_mut(&session_id) {
                session.room_id = None;
                session.pending_room_id = None;
                affected.push(session_id);
            }
        }
        affected
    }

    fn set_pending_room(&mut self, session_id: &str, room_id: Option<String>) {
        if let Some(session) = self.by_session_id.get_mut(session_id) {
            session.pending_room_id = room_id;
        }
    }

    fn remove_room_link(&mut self, session_id: &str, room_id: &str) {
        if let Some(ids) = self.by_room_id.get_mut(room_id) {
            ids.remove(session_id);
            if ids.is_empty() {
                self.by_room_id.remove(room_id);
            }
        }
    }
}

impl SessionRegistry {
    pub async fn upsert(&self, session_id: &str, session: SessionState) {
        self.inner
            .write()
            .await
            .upsert(session_id.to_owned(), session);
    }

    pub async fn get(&self, session_id: &str) -> Option<SessionState> {
        self.inner
            .read()
            .await
            .by_session_id
            .get(session_id)
            .cloned()
    }

    pub async fn remove(&self, session_id: &str) {
        self.inner.write().await.remove(session_id);
    }

    pub async fn set_room(&self, session_id: &str, room_id: Option<String>) {
        self.inner.write().await.set_room(session_id, room_id);
    }

    pub async fn set_pending_room(&self, session_id: &str, room_id: Option<String>) {
        self.inner
            .write()
            .await
            .set_pending_room(session_id, room_id);
    }

    pub async fn clear_room(&self, room_id: &str) -> Vec<String> {
        self.inner.write().await.clear_room(room_id)
    }
}
