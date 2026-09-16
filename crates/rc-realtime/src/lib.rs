//! # rc-realtime
//!
//! WebSocket hub — in-memory session registry with per-user rooms matching
//! open-webui's socket.io room model (`user:{id}`, `channel:{id}`). The HTTP
//! layer (axum) performs the upgrade + auth and hands us a sender/receiver
//! pair per session; this crate knows nothing about TCP.

use std::collections::{HashMap, HashSet};

/// Registry of connected sessions.
pub struct Hub {
    inner: parking_lot::Mutex<Inner>,
}

struct Inner {
    /// sid → session sender + user
    sessions: HashMap<u64, SessionHandle>,
    /// user_id → sids
    user_sessions: HashMap<String, HashSet<u64>>,
    next_sid: u64,
}

pub struct SessionHandle {
    pub user_id: String,
    pub tx: tokio::sync::mpsc::UnboundedSender<String>,
}

impl Hub {
    pub fn new() -> Self {
        Self {
            inner: parking_lot::Mutex::new(Inner {
                sessions: HashMap::new(),
                user_sessions: HashMap::new(),
                next_sid: 1,
            }),
        }
    }

    /// Registers an authenticated session; returns its sid and the receive
    /// half the WS handler drains toward the client.
    pub fn connect(&self, user_id: &str) -> (u64, tokio::sync::mpsc::UnboundedReceiver<String>) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let mut inner = self.inner.lock();
        let sid = inner.next_sid;
        inner.next_sid += 1;
        inner.sessions.insert(
            sid,
            SessionHandle {
                user_id: user_id.to_string(),
                tx,
            },
        );
        inner
            .user_sessions
            .entry(user_id.to_string())
            .or_default()
            .insert(sid);
        (sid, rx)
    }

    pub fn disconnect(&self, sid: u64) {
        let mut inner = self.inner.lock();
        if let Some(handle) = inner.sessions.remove(&sid)
            && let Some(set) = inner.user_sessions.get_mut(&handle.user_id)
        {
            set.remove(&sid);
            if set.is_empty() {
                inner.user_sessions.remove(&handle.user_id);
            }
        }
    }

    pub fn user_id_of(&self, sid: u64) -> Option<String> {
        self.inner
            .lock()
            .sessions
            .get(&sid)
            .map(|h| h.user_id.clone())
    }

    /// Pushes a pre-serialized frame to every open session of the user.
    /// Returns how many sessions received it (0 = user offline; callers
    /// that need durability persist to the DB regardless).
    pub fn send_to_user(&self, user_id: &str, frame: &str) -> usize {
        let inner = self.inner.lock();
        let Some(sids) = inner.user_sessions.get(user_id) else {
            return 0;
        };
        let mut sent = 0;
        for sid in sids {
            if let Some(handle) = inner.sessions.get(sid)
                && handle.tx.send(frame.to_string()).is_ok()
            {
                sent += 1;
            }
        }
        sent
    }

    /// True when any of the user's sessions is connected (usage/presence).
    pub fn is_online(&self, user_id: &str) -> bool {
        self.inner
            .lock()
            .user_sessions
            .get(user_id)
            .map(|s| !s.is_empty())
            .unwrap_or(false)
    }
}

impl Default for Hub {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 覆盖矩阵：
    // ✅ connect 返回 sid/rx；disconnect 后不再可达
    // ✅ send_to_user 只送达目标用户；多会话全达；离线 → 0
    // ✅ user_id_of / is_online 生命周期
    // ⛔ 刻意不覆盖：并发竞争（parking_lot 互斥 + 无跨锁操作）

    #[test]
    fn connect_send_disconnect_lifecycle() {
        let hub = Hub::new();
        let (sid, mut rx) = hub.connect("u1");
        assert_eq!(hub.user_id_of(sid).as_deref(), Some("u1"));
        assert!(hub.is_online("u1"));

        assert_eq!(hub.send_to_user("u1", r#"{"event":"ping"}"#), 1);
        assert_eq!(rx.try_recv().unwrap(), r#"{"event":"ping"}"#);

        hub.disconnect(sid);
        assert_eq!(hub.send_to_user("u1", "x"), 0);
        assert!(!hub.is_online("u1"));
        assert_eq!(hub.user_id_of(sid), None);
    }

    #[test]
    fn multi_session_and_isolation() {
        let hub = Hub::new();
        let (_s1, mut rx1) = hub.connect("u1");
        let (_s2, mut rx2) = hub.connect("u1");
        let (_s3, mut rx3) = hub.connect("u2");

        assert_eq!(hub.send_to_user("u1", "hello"), 2);
        assert_eq!(rx1.try_recv().unwrap(), "hello");
        assert_eq!(rx2.try_recv().unwrap(), "hello");
        assert!(rx3.try_recv().is_err(), "u2 must not receive u1 frames");

        // one of u1's sessions drops; the other still gets frames
        drop(rx1);
        hub.disconnect(_s1);
        assert_eq!(hub.send_to_user("u1", "second"), 1);
        assert_eq!(rx2.try_recv().unwrap(), "second");
    }
}
