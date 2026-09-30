//! DEFAULT_CONFIG — the open-webui 0.11.3 per-key config registry (M1
//! subset: every key queried by `/api/config` plus the auth defaults).
//! Defaults mirror `open_webui/config.py` / `env.py`. Values seeded into the
//! `config` table on first boot; the DB wins afterwards.

use std::collections::BTreeMap;

/// open-webui `DEFAULT_USER_PERMISSIONS` (config.py L1954+) — nested boolean
/// dict shipped verbatim so admin permission UIs and `get_permissions`
/// behave identically.
fn default_user_permissions() -> serde_json::Value {
    serde_json::json!({
        "workspace": {
            "models": false,
            "knowledge": false,
            "prompts": false,
            "tools": false,
            "skills": false,
            "models_import": false,
            "models_export": false,
            "prompts_import": false,
            "prompts_export": false,
            "tools_import": false,
            "tools_export": false,
            "skills_import": false,
            "skills_export": false
        },
        "sharing": {
            "models": false,
            "public_models": false,
            "knowledge": false,
            "public_knowledge": false,
            "prompts": false,
            "public_prompts": false,
            "tools": false,
            "public_tools": false,
            "skills": false,
            "public_skills": false,
            "notes": false,
            "public_notes": false,
            "folders": false,
            "public_chats": false,
            "open_chats": false,
            "public_calendars": false
        },
        "access_grants": { "allow_users": true, "allow_groups": true },
        "chat": {
            "controls": true,
            "valves": true,
            "system_prompt": true,
            "params": true,
            "file_upload": true,
            "web_upload": true,
            "delete": true,
            "delete_message": true,
            "continue_response": true,
            "regenerate_response": true,
            "rate_response": true,
            "edit": true,
            "share": true,
            "export": true,
            "import": true,
            "stt": true,
            "tts": true,
            "call": true,
            "multiple_models": true,
            "temporary": true,
            "temporary_enforced": false
        },
        "features": {
            "api_keys": false,
            "notes": true,
            "folders": true,
            "channels": true,
            "direct_tool_servers": false,
            "web_search": true,
            "image_generation": true,
            "code_interpreter": true,
            "memories": true,
            "automations": false,
            "calendar": true,
            "webhooks": true
        },
        "settings": { "interface": true }
    })
}

/// open-webui `default_prompt_suggestions` (config.py) — M1 keeps the
/// headline entries so the empty-chat screen matches.
fn default_prompt_suggestions() -> serde_json::Value {
    serde_json::json!([
        {
            "title": ["Help me study", "vocabulary for a college entrance exam"],
            "content": "Help me study vocabulary: write me sentences I can study for {{url}}"
        },
        {
            "title": ["Give me ideas", "for what to do with my kids' art"],
            "content": "Give me ideas for what to do with my kids' art"
        },
        {
            "title": ["Tell me a fun fact", "about the Roman Empire"],
            "content": "Tell me a random fun fact about the Roman Empire"
        },
        {
            "title": ["Show me a code snippet", "of a website's sticky header"],
            "content": "Show me a code snippet of a website's sticky header"
        },
        {
            "title": ["Explain options trading", "if I'm familiar with buying and selling stocks"],
            "content": "Explain options trading if I'm familiar with buying and selling stocks"
        },
        {
            "title": ["Overcome procrastination", "give me tips"],
            "content": "Overcome procrastination and give me tips to get better at it"
        }
    ])
}

pub fn default_config() -> BTreeMap<String, serde_json::Value> {
    let mut m = BTreeMap::new();

    // --- auth (M1-relevant) ---
    m.insert("auth.jwt_expiry".into(), serde_json::json!("4w"));
    m.insert("auth.enable_api_keys".into(), serde_json::json!(false));
    m.insert(
        "auth.api_key.allowed_endpoints".into(),
        serde_json::json!(""),
    );
    m.insert(
        "auth.api_key.endpoint_restrictions".into(),
        serde_json::json!(false),
    );

    // --- oauth / ldap (M7, queried by /api/config) ---
    m.insert("oauth.enable".into(), serde_json::json!(true));
    m.insert("oauth.auto_redirect".into(), serde_json::json!(false));
    m.insert("ldap.enable".into(), serde_json::json!(false));

    // --- ui ---
    m.insert("ui.enable_signup".into(), serde_json::json!(true));
    m.insert("ui.enable_login_form".into(), serde_json::json!(true));
    m.insert(
        "ui.enable_password_change_form".into(),
        serde_json::json!(true),
    );
    m.insert(
        "ui.enable_community_sharing".into(),
        serde_json::json!(true),
    );
    m.insert("ui.enable_message_rating".into(), serde_json::json!(true));
    m.insert("ui.enable_user_webhooks".into(), serde_json::json!(true));
    m.insert("ui.default_user_role".into(), serde_json::json!("pending"));
    m.insert("ui.default_models".into(), serde_json::json!(null));
    m.insert("ui.default_pinned_models".into(), serde_json::json!(null));
    m.insert(
        "ui.default_interface_settings".into(),
        serde_json::json!({}),
    );
    m.insert("ui.default_group_id".into(), serde_json::json!(null));
    m.insert("ui.prompt_suggestions".into(), default_prompt_suggestions());
    m.insert(
        "ui.pending_user_overlay_title".into(),
        serde_json::json!("Account pending"),
    );
    m.insert(
        "ui.pending_user_overlay_content".into(),
        serde_json::json!("Wait for admin approval."),
    );
    m.insert("ui.watermark".into(), serde_json::json!(true));

    // --- feature toggles (M2+ subsystems; M1 ships them off like fresh OWU) ---
    m.insert("folders.enable".into(), serde_json::json!(true));
    m.insert("folders.max_file_count".into(), serde_json::json!(null));
    m.insert("channels.enable".into(), serde_json::json!(false));
    m.insert("calendar.enable".into(), serde_json::json!(true));
    m.insert("automations.enable".into(), serde_json::json!(true));
    m.insert("notes.enable".into(), serde_json::json!(true));
    m.insert(
        "chat.context_compaction.enable".into(),
        serde_json::json!(false),
    );
    m.insert(
        "chat.tool_permissions.enable".into(),
        serde_json::json!(true),
    );
    m.insert("web.search.enable".into(), serde_json::json!(false));
    m.insert(
        "web.search.confirmation.enable".into(),
        serde_json::json!(false),
    );
    m.insert(
        "web.search.confirmation.content".into(),
        serde_json::json!(""),
    );
    m.insert("code_execution.enable".into(), serde_json::json!(false));
    m.insert("code_execution.engine".into(), serde_json::json!("pyodide"));
    m.insert("code_interpreter.enable".into(), serde_json::json!(true));
    m.insert(
        "code_interpreter.engine".into(),
        serde_json::json!("pyodide"),
    );
    m.insert("image_generation.enable".into(), serde_json::json!(false));
    m.insert("task.autocomplete.enable".into(), serde_json::json!(false));

    // --- task models (title generation family; M5 adds tags/queries/…) ---
    // open-webui: TASK_MODEL / TASK_MODEL_EXTERNAL env, empty = unset.
    let env = |key: &str| std::env::var(key).unwrap_or_default();
    m.insert(
        "task.model.default".into(),
        serde_json::json!(env("TASK_MODEL")),
    );
    m.insert(
        "task.model.external".into(),
        serde_json::json!(env("TASK_MODEL_EXTERNAL")),
    );
    m.insert("task.model.params".into(), serde_json::json!({}));
    m.insert(
        "task.title.enable".into(),
        serde_json::json!(
            std::env::var("ENABLE_TITLE_GENERATION")
                .unwrap_or_else(|_| "True".into())
                .to_lowercase()
                == "true"
        ),
    );
    m.insert("task.title.prompt_template".into(), serde_json::json!(""));
    m.insert("google_drive.enable".into(), serde_json::json!(false));
    m.insert("onedrive.enable".into(), serde_json::json!(false));
    m.insert("memories.enable".into(), serde_json::json!(true));
    m.insert("users.enable_status".into(), serde_json::json!(true));

    // --- media / rag placeholders (M2/M5 keys queried by /api/config) ---
    m.insert("audio.tts.engine".into(), serde_json::json!(""));
    m.insert("audio.tts.voice".into(), serde_json::json!(""));
    m.insert(
        "audio.tts.split_on".into(),
        serde_json::json!("punctuation"),
    );
    m.insert("audio.stt.engine".into(), serde_json::json!(""));
    m.insert("rag.file.max_size".into(), serde_json::json!(null));
    m.insert("rag.file.max_count".into(), serde_json::json!(null));
    m.insert(
        "file.image_compression_width".into(),
        serde_json::json!(null),
    );
    m.insert(
        "file.image_compression_height".into(),
        serde_json::json!(null),
    );

    // --- backends (M1-4) ---
    m.insert("ollama.enable".into(), serde_json::json!(false));
    m.insert("ollama.base_urls".into(), serde_json::json!([]));
    m.insert("openai.enable".into(), serde_json::json!(false));
    m.insert("openai.api_base_urls".into(), serde_json::json!([]));
    m.insert("openai.api_keys".into(), serde_json::json!([]));
    m.insert("openai.api_configs".into(), serde_json::json!([]));
    m.insert("direct.enable".into(), serde_json::json!(false));

    // --- permissions ---
    m.insert("user.permissions".into(), default_user_permissions());

    m
}
