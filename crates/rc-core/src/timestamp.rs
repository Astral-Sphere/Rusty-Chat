//! Timestamp policy — the single most dangerous compatibility detail.
//!
//! open-webui 0.11.3 stores unix epochs in **two different units** depending
//! on the table, and `chat.timer_at` even differs from `chat.created_at` on
//! the *same row*:
//!
//! | unit | tables |
//! |---|---|
//! | seconds | `user`, `auth`(none), `api_key`, `chat` (created/updated/last_read_at), `chat_message`, `chat_file`, `shared_chat`, `tag`, `memory`, `access_grant`, `config`, `tool`, `function`, `model`, `file`, `folder`, `prompt`, `prompt_history`, `feedback`, `knowledge*`, `group`, `group_member`, `oauth_session`, `skill` |
//! | nanoseconds | `message`, `message_reaction`, `channel`, `channel_member`, `channel_file`, `channel_webhook`, `note`, `pinned_note`, `automation`, `automation_run`, `calendar`, `calendar_event`, `calendar_event_attendee`, `chat.timer_at` |
//!
//! Python source: `int(time.time())` vs `int(time.time_ns())` in
//! `open-webui/backend/open_webui/models/*.py`.
//!
//! Rusty-Chat encodes this in the type system: repositories accept and return
//! [`Secs`] or [`Nanos`], never bare `i64`, so a mixed-up unit fails to
//! compile instead of silently corrupting a shared database.

use serde::{Deserialize, Serialize};

macro_rules! epoch_unit {
    ($name:ident, $doc:literal) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        #[doc = $doc]
        pub struct $name(pub i64);

        impl $name {
            #[doc = concat!("Current time as `", stringify!($name), "`.")]
            pub fn now() -> Self {
                Self(Self::now_i64())
            }

            #[doc = concat!("Builds from a bare i64 without conversion — use only for values read from the DB.")]
            pub const fn from_raw(raw: i64) -> Self {
                Self(raw)
            }

            pub const fn as_i64(self) -> i64 {
                self.0
            }

            pub fn to_owned_string(self) -> String {
                self.0.to_string()
            }
        }

        impl From<$name> for i64 {
            fn from(v: $name) -> i64 {
                v.0
            }
        }
    };
}

epoch_unit!(Secs, "Unix epoch in **seconds** (`int(time.time())`).");
epoch_unit!(
    Nanos,
    "Unix epoch in **nanoseconds** (`int(time.time_ns())`)."
);

impl Secs {
    fn now_i64() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before 1970")
            .as_secs() as i64
    }

    /// Converts from nanoseconds, truncating sub-second precision — mirrors
    /// Python `int(time.time())` on the same wall clock.
    pub fn from_nanos_floor(nanos: Nanos) -> Self {
        Self(nanos.0.div_euclid(1_000_000_000))
    }
}

impl Nanos {
    fn now_i64() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before 1970")
            .as_nanos() as i64
    }

    /// Converts from seconds — mirrors `time.time_ns()` semantics.
    pub fn from_secs(secs: Secs) -> Self {
        Self(secs.0.saturating_mul(1_000_000_000))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-09-15 falls in this range; anything outside means we mixed units.
    const SECS_MIN: i64 = 1_700_000_000; // 2023-11
    const SECS_MAX: i64 = 2_000_000_000; // 2033-05
    const NANOS_MIN: i64 = SECS_MIN * 1_000_000_000;
    const NANOS_MAX: i64 = SECS_MAX * 1_000_000_000;

    #[test]
    fn secs_now_is_in_seconds_range() {
        let now = Secs::now();
        assert!(SECS_MIN <= now.0 && now.0 <= SECS_MAX, "got {}", now.0);
    }

    #[test]
    fn nanos_now_is_in_nanoseconds_range() {
        let now = Nanos::now();
        assert!(NANOS_MIN <= now.0 && now.0 <= NANOS_MAX, "got {}", now.0);
    }

    #[test]
    fn conversions_roundtrip() {
        let secs = Secs(1_757_890_000);
        let nanos = Nanos::from_secs(secs);
        assert_eq!(nanos.0, 1_757_890_000_000_000_000);
        assert_eq!(Secs::from_nanos_floor(nanos), secs);
    }

    #[test]
    fn from_nanos_floor_truncates_toward_negative_infinity() {
        // Python int() truncates toward zero; epochs are positive in practice,
        // but div_euclid keeps the math honest if a pre-1970 value appears.
        assert_eq!(Secs::from_nanos_floor(Nanos(1_999_999_999)), Secs(1));
        assert_eq!(Secs::from_nanos_floor(Nanos(-1)), Secs(-1));
    }

    #[test]
    fn serde_is_transparent_i64() {
        // DB stores bare integers; JSON payloads carry bare integers too.
        assert_eq!(serde_json::to_string(&Secs(123)).unwrap(), "123");
        assert_eq!(serde_json::to_string(&Nanos(456)).unwrap(), "456");
        assert_eq!(serde_json::from_str::<Secs>("123").unwrap(), Secs(123));
        assert_eq!(serde_json::from_str::<Nanos>("456").unwrap(), Nanos(456));
    }
}
