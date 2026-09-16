//! `GET /api/config` — public app configuration. JSON shape mirrors
//! open-webui `get_app_config` (M1 key set; every queried config key exists
//! in `defaults::default_config`).

use axum::Json;
use axum::extract::State;
use axum::http::request::Parts;
use rc_db::repo::users;
use serde_json::{Value, json};

use crate::extract::optional_user;
use crate::state::{APP_CONFIG_KEYS, AppState};

/// Fetch all /api/config keys; missing rows fall back to `defaults`.
pub async fn fetch_config_map(app: &AppState) -> std::collections::BTreeMap<String, Value> {
    let mut out = std::collections::BTreeMap::new();
    let defaults = crate::defaults::default_config();
    for key in APP_CONFIG_KEYS {
        let value = app
            .config
            .get(key)
            .await
            .ok()
            .flatten()
            .or_else(|| defaults.get(*key).cloned())
            .unwrap_or(Value::Null);
        out.insert((*key).to_string(), value);
    }
    out
}

pub async fn get_app_config(State(app): State<AppState>, mut parts: Parts) -> Json<Value> {
    let user = optional_user(&mut parts, &app).await;
    let config = fetch_config_map(&app).await;

    let get = |key: &str| config.get(key).cloned().unwrap_or(Value::Null);
    let get_bool = |key: &str| config.get(key).and_then(Value::as_bool).unwrap_or(false);

    let onboarding = if user.is_none() {
        users::get_num_users(&app.db).await.unwrap_or(0) == 0
    } else {
        false
    };

    let mut features = json!({
        "auth": app.webui_auth,
        "auth_trusted_header": false,
        "enable_signup_password_confirmation": false,
        "enable_ldap": get("ldap.enable"),
        "enable_signup": get("ui.enable_signup"),
        "enable_login_form": get("ui.enable_login_form"),
        "enable_websocket": true,
        "websocket_heartbeat_interval": 0,
        // authenticated-only flags (open-webui nests them unconditionally in 0.11)
        "enable_api_keys": get("auth.enable_api_keys"),
        "enable_password_change_form": get("ui.enable_password_change_form"),
        "enable_version_update_check": false,
        "enable_pyodide_file_persistence": false,
        "enable_public_active_users_count": false,
        "enable_easter_eggs": true,
        "enable_direct_connections": get("direct.enable"),
        "enable_folders": get("folders.enable"),
        "enable_channels": get("channels.enable"),
        "enable_calendar": get("calendar.enable"),
        "enable_automations": get("automations.enable"),
        "enable_notes": get("notes.enable"),
        "enable_autocomplete_generation": get("task.autocomplete.enable"),
        "enable_web_search": get("web.search.enable"),
        "enable_web_search_query_generation": get_bool("web.search.enable"),
        "enable_web_search_confirmation": get("web.search.confirmation.enable"),
        "enable_code_execution": get("code_execution.enable"),
        "enable_code_interpreter": get("code_interpreter.enable"),
        "enable_image_generation": get("image_generation.enable"),
        "enable_memory": get("memories.enable"),
        "enable_community_sharing": get("ui.enable_community_sharing"),
        "enable_message_rating": get("ui.enable_message_rating"),
        "enable_user_webhooks": get("ui.enable_user_webhooks"),
        "enable_user_status": get("users.enable_status"),
        "enable_google_drive_integration": get("google_drive.enable"),
        "enable_onedrive_integration": get("onedrive.enable"),
        "enable_admin_analytics": false,
    });

    // authenticated users see the richer feature set; anonymous requests get
    // only the public subset (open-webui gates on `user is not None`).
    if user.is_none() {
        let public = json!({
            "auth": features["auth"],
            "auth_trusted_header": features["auth_trusted_header"],
            "enable_signup_password_confirmation": features["enable_signup_password_confirmation"],
            "enable_ldap": features["enable_ldap"],
            "enable_signup": features["enable_signup"],
            "enable_login_form": features["enable_login_form"],
            "enable_websocket": features["enable_websocket"],
        });
        features = public;
    }

    let mut body = json!({
        "status": true,
        "name": app.webui_name,
        "version": app.version,
        "default_locale": "en-US",
        "oauth": {
            "providers": {},
            "auto_redirect": get("oauth.auto_redirect"),
        },
        "features": features,
        "default_models": get("ui.default_models"),
        "default_pinned_models": get("ui.default_pinned_models"),
        "default_prompt_suggestions": get("ui.prompt_suggestions"),
        "code": {
            "engine": get("code_interpreter.engine"),
        },
        "audio": {
            "tts": {
                "engine": get("audio.tts.engine"),
                "voice": get("audio.tts.voice"),
                "split_on": get("audio.tts.split_on"),
            },
            "stt": {
                "engine": get("audio.stt.engine"),
            }
        },
        "file": {
            "max_size": get("rag.file.max_size"),
            "max_count": get("rag.file.max_count"),
            "image_compression_width": get("file.image_compression_width"),
            "image_compression_height": get("file.image_compression_height"),
        },
        "permissions": get("user.permissions"),
        "watermark": get("ui.watermark"),
        "ui": {
            "pending_user_overlay_title": get("ui.pending_user_overlay_title"),
            "pending_user_overlay_content": get("ui.pending_user_overlay_content"),
            "default_interface_settings": get("ui.default_interface_settings"),
        },
    });
    if onboarding {
        body["onboarding"] = json!(true);
    }
    Json(body)
}
