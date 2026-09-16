//! Shared application state.

use rc_db::repo::config::ConfigEngine;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub db: sea_orm::DatabaseConnection,
    pub config: Arc<ConfigEngine>,
    /// `WEBUI_SECRET_KEY`
    pub secret_key: String,
    pub webui_name: String,
    pub version: &'static str,
    /// bcrypt placeholder hash for unknown-user sign-ins (timing defense).
    pub placeholder_hash: Arc<String>,
    /// open-webui `WEBUI_AUTH` (M1: always true in effect).
    pub webui_auth: bool,
    /// WebSocket session registry (rooms user:{id}).
    pub hub: Arc<rc_realtime::Hub>,
}

/// Config keys fetched for every `/api/config` request, in the exact order
/// open-webui's `Config.get_many` call lists them (value set matches
/// `get_app_config`).
pub const APP_CONFIG_KEYS: &[&str] = &[
    "oauth.enable",
    "oauth.auto_redirect",
    "ldap.enable",
    "ui.enable_signup",
    "ui.enable_login_form",
    "auth.enable_api_keys",
    "ui.enable_password_change_form",
    "direct.enable",
    "folders.enable",
    "folders.max_file_count",
    "channels.enable",
    "calendar.enable",
    "automations.enable",
    "notes.enable",
    "chat.context_compaction.enable",
    "chat.tool_permissions.enable",
    "web.search.enable",
    "web.search.confirmation.enable",
    "web.search.confirmation.content",
    "code_execution.enable",
    "code_interpreter.enable",
    "image_generation.enable",
    "task.autocomplete.enable",
    "ui.enable_community_sharing",
    "ui.enable_message_rating",
    "ui.enable_user_webhooks",
    "users.enable_status",
    "google_drive.enable",
    "onedrive.enable",
    "memories.enable",
    "ui.default_models",
    "ui.default_pinned_models",
    "ui.default_interface_settings",
    "ui.prompt_suggestions",
    "code_execution.engine",
    "code_interpreter.engine",
    "audio.tts.engine",
    "audio.tts.voice",
    "audio.tts.split_on",
    "audio.stt.engine",
    "rag.file.max_size",
    "rag.file.max_count",
    "file.image_compression_width",
    "file.image_compression_height",
    "user.permissions",
    "ui.pending_user_overlay_title",
    "ui.pending_user_overlay_content",
    "ui.watermark",
];
