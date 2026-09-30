//! Rusty-Chat Dioxus CSR frontend — M1-6: login (signin/signup), chat list,
//! streaming chat over the native WebSocket.
//!
//! M1 scope notes (see docs/PROGRESS.md M1-6):
//! - messages render as preformatted plain text during streaming; the
//!   comrak+katex-rs+tree-sitter markdown pipeline is the next increment.
//! - one app-level WebSocket; events filter by chat_id.

mod api;
mod branches;
mod highlight;
mod mermaid;
mod render;

use dioxus::prelude::*;
use serde_json::{Value, json};

#[derive(Clone, Debug)]
struct ChatEntry {
    id: String,
    title: String,
    active: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct ChatMessageState {
    id: String,
    role: String,
    content: String,
    done: bool,
    is_error: bool,
    // ‹ › branch switcher: 1-based position among siblings (0 when hidden)
    sibling_index: usize,
    sibling_count: usize,
}

impl ChatMessageState {
    fn plain(id: String, role: &str, content: String, done: bool) -> Self {
        Self {
            id,
            role: role.to_string(),
            content,
            done,
            is_error: false,
            sibling_index: 0,
            sibling_count: 0,
        }
    }
}

fn main() {
    launch(app);
}

fn app() -> Element {
    // Generated from web/input.css by the Tailwind v4 standalone CLI
    // (`just web-css`; gitignored, regenerate once after a fresh clone).
    let main_css = asset!("/assets/tailwind.css");
    let katex_css = asset!("/assets/katex/katex.min.css");
    let mut token = use_signal(api::token);
    let generation_active = use_signal(|| false);
    // bumped when a background task changes chat state (title generation)
    let list_refresh = use_signal(|| 0u32);
    let selected_chat = use_signal(|| None::<String>);

    rsx! {
        document::Link { rel: "stylesheet", href: main_css }
        document::Link { rel: "stylesheet", href: katex_css }
        if token().is_none() {
            login_view { on_signed_in: move |t| { token.set(Some(t)); } }
        } else {
            div { class: "flex h-screen",
                chat_list_view {
                    token: token().unwrap_or_default(),
                    generation_active: generation_active(),
                    refresh: list_refresh,
                    selected: selected_chat,
                    on_sign_out: move |_| {
                        api::clear_token();
                        token.set(None);
                    },
                }
                chat_view {
                    token: token().unwrap_or_default(),
                    generation_active: generation_active,
                    list_refresh: list_refresh,
                    selected: selected_chat,
                }
            }
        }
    }
}

#[component]
fn login_view(on_signed_in: EventHandler<String>) -> Element {
    let mut mode_signup = use_signal(|| false);
    let mut email = use_signal(String::new);
    let mut password = use_signal(String::new);
    let mut name = use_signal(String::new);
    let error = use_signal(String::new);
    let busy = use_signal(|| false);

    let submit = move |_| {
        to_owned![
            mode_signup,
            email,
            password,
            name,
            error,
            busy,
            on_signed_in
        ];
        async move {
            busy.set(true);
            error.set(String::new());
            let path = if mode_signup() {
                "/api/v1/auths/signup"
            } else {
                "/api/v1/auths/signin"
            };
            let mut body = json!({"email": email(), "password": password()});
            if mode_signup() {
                body["name"] = json!(name());
            }
            match api::api_post(path, &body).await {
                Ok((200, response)) => {
                    if let Some(token) = response["token"].as_str() {
                        api::set_token(token);
                        on_signed_in.call(token.to_string());
                        return;
                    }
                    error.set("unexpected response".into());
                }
                Ok((_, response)) => {
                    let detail = response["detail"].as_str().unwrap_or("login failed");
                    error.set(detail.to_string());
                }
                Err(e) => error.set(e),
            }
            busy.set(false);
        }
    };

    rsx! {
        div { class: "min-h-screen flex items-center justify-center bg-gray-950 text-gray-100",
            div { class: "w-96 space-y-4",
                h1 { class: "text-2xl font-semibold text-center", "Rusty-Chat" }
                div { class: "flex gap-2 justify-center",
                    button {
                        class: if !mode_signup() { "px-3 py-1 rounded bg-blue-600" } else { "px-3 py-1 rounded bg-gray-800" },
                        onclick: move |_| mode_signup.set(false),
                        "Sign in"
                    }
                    button {
                        class: if mode_signup() { "px-3 py-1 rounded bg-blue-600" } else { "px-3 py-1 rounded bg-gray-800" },
                        onclick: move |_| mode_signup.set(true),
                        "Sign up"
                    }
                }
                if mode_signup() {
                    input {
                        class: "w-full px-3 py-2 rounded bg-gray-800",
                        placeholder: "Name",
                        value: name(),
                        oninput: move |e| name.set(e.value()),
                    }
                }
                input {
                    class: "w-full px-3 py-2 rounded bg-gray-800",
                    placeholder: "Email",
                    value: email(),
                    oninput: move |e| email.set(e.value()),
                }
                input {
                    class: "w-full px-3 py-2 rounded bg-gray-800",
                    placeholder: "Password",
                    r#type: "password",
                    value: password(),
                    oninput: move |e| password.set(e.value()),
                }
                if !error().is_empty() {
                    div { class: "text-red-400 text-sm", "{error}" }
                }
                button {
                    class: "w-full px-3 py-2 rounded bg-blue-600 disabled:opacity-50",
                    disabled: busy(),
                    onclick: submit,
                    if busy() { "Working…" } else { "Continue" }
                }
            }
        }
    }
}

#[component]
fn chat_list_view(
    token: String,
    generation_active: bool,
    // bumped by chat_view when a `chat:title` event rewrites a title
    refresh: Signal<u32>,
    // shared selection: set by the sidebar, consumed by chat_view
    selected: Signal<Option<String>>,
    on_sign_out: EventHandler<()>,
) -> Element {
    let chats = use_signal(Vec::<ChatEntry>::new);
    let mut selected_id = selected;
    let reload_nonce = use_signal(|| 0u32);

    use_effect(move || {
        reload_nonce();
        refresh();
        to_owned![chats];
        spawn(async move {
            if let Ok(list) = api::api_get("/api/v1/chats/").await {
                let entries: Vec<ChatEntry> = list
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|c| {
                                Some(ChatEntry {
                                    id: c["id"].as_str()?.to_string(),
                                    title: c["title"].as_str().unwrap_or("New Chat").to_string(),
                                    active: c["active"].as_bool().unwrap_or(false),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                chats.set(entries);
            }
        });
    });

    rsx! {
        aside { class: "w-72 bg-gray-900 text-gray-200 flex flex-col",
            div { class: "p-3 flex items-center justify-between",
                span { class: "font-semibold", "Chats" }
                button {
                    class: "text-sm px-2 py-1 rounded bg-gray-800",
                    onclick: move |_| {
                        // clear current selection so chat_view starts a new chat
                        selected_id.set(None);
                    },
                    "+ New"
                }
            }
            ul { class: "flex-1 overflow-y-auto",
                for entry in chats() {
                    li {
                        key: "{entry.id}",
                        class: if selected_id() == Some(entry.id.clone()) { "px-3 py-2 cursor-pointer bg-gray-700" } else { "px-3 py-2 cursor-pointer hover:bg-gray-800" },
                        onclick: move |_| selected_id.set(Some(entry.id.clone())),
                        "{entry.title}"
                        if entry.active { span { class: "ml-2 text-blue-400", "•" } }
                    }
                }
            }
            div { class: "p-3 border-t border-gray-800",
                button {
                    class: "text-sm text-gray-400",
                    onclick: move |_| {
                        spawn(async move {
                            api::api_post("/api/v1/auths/signout", &json!({})).await.ok();
                            on_sign_out.call(());
                        });
                    },
                    "Sign out"
                }
                span { class: "ml-2 text-xs text-gray-600", if generation_active { "generating…" } }
            }
        }
    }
}

#[component]
fn chat_view(
    token: String,
    generation_active: Signal<bool>,
    list_refresh: Signal<u32>,
    selected: Signal<Option<String>>,
) -> Element {
    let models = use_signal(Vec::<String>::new);
    let mut selected_model = use_signal(String::new);
    let mut messages = use_signal(Vec::<ChatMessageState>::new);
    let mut history = use_signal(Value::default);
    let mut input = use_signal(String::new);
    let mut chat_id = use_signal(|| None::<String>);
    let mut generation_done_nonce = use_signal(|| 0u32);

    // sidebar selection drives the open chat; None starts a new chat
    use_effect(move || {
        let picked = selected();
        if picked != chat_id() {
            chat_id.set(picked);
            messages.set(Vec::new());
            history.set(Value::default());
            generation_done_nonce += 1;
        }
    });

    // load model list once
    use_effect(move || {
        to_owned![models, selected_model];
        spawn(async move {
            if let Ok(value) = api::api_get("/api/models").await {
                let ids: Vec<String> = value["data"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|m| m["id"].as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                if !ids.is_empty() {
                    selected_model.set(ids[0].clone());
                }
                models.set(ids);
            }
        });
    });

    // websocket lifecycle: reconnect when the token changes
    use_effect(move || {
        to_owned![messages, generation_done_nonce, list_refresh];
        spawn(async move {
            let Some(token) = api::token() else { return };
            api::WsChannel::connect(token, move |frame| {
                if frame["event"] != json!("events") {
                    return;
                }
                let data = &frame["data"]["data"];
                let message_id = frame["data"]["message_id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                match data["type"].as_str().unwrap_or_default() {
                    "chat:message:delta" => {
                        let content = data["data"]["content"].as_str().unwrap_or_default();
                        append_delta(messages, &message_id, content);
                    }
                    "chat:message" => {
                        if data["data"].get("done") == Some(&json!(true)) {
                            finalize_message(messages, &message_id, data);
                            generation_done_nonce += 1;
                        }
                    }
                    "chat:message:error" => {
                        mark_error(messages, &message_id);
                        generation_done_nonce += 1;
                    }
                    "chat:active" => {
                        let active = data["data"]["active"].as_bool().unwrap_or(false);
                        generation_active.set(active);
                    }
                    "chat:title" => {
                        // sidebar reloads and picks up the new title
                        list_refresh += 1;
                    }
                    _ => {}
                }
            });
        });
    });

    // reload the message list when generation finishes (server persisted);
    // display follows the active branch path of history.currentId
    use_effect(move || {
        generation_done_nonce();
        to_owned![messages, chat_id, history];
        spawn(async move {
            if let Some(id) = chat_id()
                && let Ok(chat) = api::api_get(&format!("/api/v1/chats/{id}")).await
            {
                let blob_history = chat["chat"]["history"].clone();
                history.set(blob_history.clone());
                messages.set(build_view(&blob_history));
            }
        });
    });

    let send = move |_| {
        to_owned![input, messages, selected_model, chat_id, selected, history];
        async move {
            let content = input().trim().to_string();
            web_sys::console::log_1(&format!("SEND sig={:?} model={:?}", input(), selected_model()).into());
            if content.is_empty() || selected_model().is_empty() {
                web_sys::console::log_1(&"SEND early-return".into());
                return;
            }
            input.set(String::new());

            let assistant_id = api::uuid_v4();
            let user_message_id = api::uuid_v4();
            let this_chat = chat_id().unwrap_or_else(api::uuid_v4);
            // keep the shared selection in lockstep (adjacent sets — the
            // selection effect observes them together and stays a no-op)
            selected.set(Some(this_chat.clone()));
            chat_id.set(Some(this_chat.clone()));

            // open-webui submitPrompt semantics: a follow-up parents onto
            // history.currentId and the LLM receives the full active chain
            let parent: Option<String> = history()
                .get("currentId")
                .and_then(Value::as_str)
                .filter(|p| !p.is_empty())
                .map(str::to_string);
            let chain =
                branches::messages_for_regeneration(&history(), parent.as_deref(), &content);

            // local tree patch (mirrors the server upsert), then re-project
            // the view from the active path — keeps everything in position
            let mut local = history();
            let now = chrono_secs();
            branches::attach_user_message(
                &mut local,
                parent.as_deref(),
                &user_message_id,
                &content,
                now,
            );
            branches::attach_assistant_placeholder(
                &mut local,
                &user_message_id,
                &assistant_id,
                now,
            );
            history.set(local.clone());
            messages.set(build_view(&local));

            let body = json!({
                "model": selected_model(),
                "messages": chain,
                "stream": true,
                "id": assistant_id,
                "parent_id": parent,
                "chat_id": this_chat,
                "user_message": {
                    "id": user_message_id,
                    "parentId": parent,
                    "role": "user",
                    "content": content,
                },
                "session_id": null,
            });
            if let Ok((status, response)) = api::api_post("/api/chat/completions", &body).await
                && status != 200
            {
                let detail = response["detail"]
                    .as_str()
                    .unwrap_or("request failed")
                    .to_string();
                mark_error(messages, &assistant_id);
                let _ = detail;
                generation_done_nonce += 1;
            }
        }
    };

    // switch to a sibling branch: currentId lands on the sibling's LEAF
    // (open-webui semantics) so the whole branch shows, then reload
    let on_switch = {
        to_owned![chat_id, history, generation_done_nonce];
        move |target_id: String| {
            spawn(async move {
                let Some(chat) = chat_id() else { return };
                let leaf = branches::leaf_descendant(&history(), &target_id);
                let _ = api::api_post(
                    &format!("/api/v1/chats/{chat}"),
                    &json!({"chat": {"history": {"currentId": leaf}}}),
                )
                .await;
                generation_done_nonce += 1;
            });
        }
    };

    // edit a user message → sibling branch with a fresh assistant response.
    // open-webui editMessage semantics: patch the tree LOCALLY (new sibling
    // under the old message's parent, currentId moves to the new branch) and
    // re-project the view from the active path — so the edited message
    // REPLACES the old one in place instead of appearing at the bottom.
    let on_edit_save = {
        to_owned![
            history,
            chat_id,
            selected_model,
            messages,
            generation_done_nonce
        ];
        move |(old_user_id, content): (String, String)| {
            spawn(async move {
                let Some(this_chat) = chat_id() else { return };
                if content.trim().is_empty() {
                    return;
                }
                let old = history()["messages"][old_user_id.as_str()].clone();
                let parent: Option<String> = old["parentId"]
                    .as_str()
                    .filter(|p| !p.is_empty())
                    .map(str::to_string);
                let chain =
                    branches::messages_for_regeneration(&history(), parent.as_deref(), &content);
                let new_user_id = api::uuid_v4();
                let new_assistant_id = api::uuid_v4();

                let mut local = history();
                let now = chrono_secs();
                branches::attach_user_message(
                    &mut local,
                    parent.as_deref(),
                    &new_user_id,
                    &content,
                    now,
                );
                branches::attach_assistant_placeholder(
                    &mut local,
                    &new_user_id,
                    &new_assistant_id,
                    now,
                );
                history.set(local.clone());
                messages.set(build_view(&local));

                let body = json!({
                    "model": selected_model(),
                    "messages": chain,
                    "stream": true,
                    "id": new_assistant_id,
                    "parent_id": new_user_id,
                    "chat_id": this_chat,
                    "user_message": {
                        "id": new_user_id,
                        "parentId": parent,
                        "role": "user",
                        "content": content,
                    },
                });
                if let Ok((status, response)) = api::api_post("/api/chat/completions", &body).await
                    && status != 200
                {
                    let _ = response;
                    generation_done_nonce += 1;
                }
            });
        }
    };

    rsx! {
        main { class: "flex-1 flex flex-col bg-gray-950 text-gray-100",
            div { class: "p-2 border-b border-gray-800",
                select {
                    class: "bg-gray-800 px-2 py-1 rounded text-sm",
                    value: selected_model(),
                    onchange: move |e| selected_model.set(e.value()),
                    for id in models() {
                        option { value: "{id}", "{id}" }
                    }
                }
            }
            div { class: "flex-1 overflow-y-auto p-4 space-y-3",
                for message in messages() {
                    // Keyed on id+done+content hash so the memoized markdown
                    // render recomputes only when the message actually changes
                    // (streaming flip, finalize overwrite, server reload).
                    message_item {
                        key: "{message_key(&message)}",
                        message: message.clone(),
                        history: history(),
                        on_switch,
                        on_edit_save,
                    }
                }
            }
            div { class: "p-3 border-t border-gray-800",
                input {
                    class: "w-full px-3 py-2 rounded bg-gray-800",
                    placeholder: if generation_active() { "generating…" } else { "Message Rusty-Chat" },
                    value: input(),
                    oninput: move |e| {
                        input.set(e.value());
                        web_sys::console::log_1(&format!("ONINPUT sig={:?}", input()).into());
                    },
                    onkeydown: move |e| {
                        if e.key() == Key::Enter {
                            web_sys::console::log_1(&"KD-ENTER".into());
                            spawn(send(()));
                        }
                    },
                }
            }
        }
    }
}

/// Projects the display list from the blob history's active branch path
/// (root → currentId leaf), each message annotated with its sibling switcher
/// position. The ONLY source of message order — mutations patch `history`
/// and re-project, never push onto the list directly.
fn build_view(history: &Value) -> Vec<ChatMessageState> {
    branches::active_path(history)
        .iter()
        .map(|id| {
            let m = &history["messages"][id.as_str()];
            let (sibling_index, sibling_count) = branches::sibling_position(history, id);
            ChatMessageState {
                id: id.clone(),
                role: m["role"].as_str().unwrap_or("user").to_string(),
                content: m["content"].as_str().unwrap_or_default().to_string(),
                done: m["done"].as_bool().unwrap_or(true),
                is_error: false,
                sibling_index,
                sibling_count,
            }
        })
        .collect()
}

/// Unix epoch seconds (matches the server's message timestamps).
fn chrono_secs() -> i64 {
    // std::time::SystemTime::now() is unimplemented on
    // wasm32-unknown-unknown and panics — read the wall clock from JS.
    (js_sys::Date::now() / 1000.0) as i64
}

/// Stable per-message key: id + done + content hash. A content change (finalize
/// overwrite, server reload, later edit) creates a fresh component instance so
/// the memoized markdown render inside recomputes.
fn message_key(message: &ChatMessageState) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    message.content.hash(&mut hasher);
    format!("{}-{}-{:016x}", message.id, message.done, hasher.finish())
}

#[component]
fn message_item(
    message: ChatMessageState,
    history: Value,
    on_switch: EventHandler<String>,
    on_edit_save: EventHandler<(String, String)>,
) -> Element {
    // The instance key (see message_key) guarantees content is stable for this
    // component's lifetime, so a plain once-computed memo is enough.
    let content = message.content.clone();
    let rendered = use_memo(move || render::render_markdown(&content));
    // after a done render: mermaid fences become SVGs (or keep their source
    // plus an error note), then code fences get syntax-highlight spans
    use_effect(move || {
        rendered();
        spawn(async move {
            mermaid::render_mermaid_blocks().await;
            highlight::backfill_code_blocks().await;
        });
    });

    // local edit state (user messages only)
    let mut edit_mode = use_signal(|| false);
    let mut edit_text = use_signal(String::new);

    let align = if message.role == "user" {
        "ml-auto max-w-xl"
    } else {
        "mr-auto max-w-2xl"
    };

    let bubble = if message.is_error {
        rsx! {
            div { class: "max-w-xl px-3 py-2 rounded bg-red-900 whitespace-pre-wrap",
                "{message.content}"
            }
        }
    } else if edit_mode() {
        let message_id = message.id.clone();
        rsx! {
            div { class: "{align} w-full space-y-2",
                textarea {
                    class: "w-full px-3 py-2 rounded bg-gray-800",
                    value: edit_text(),
                    rows: 3,
                    oninput: move |e| edit_text.set(e.value()),
                }
                div { class: "flex gap-2 justify-end",
                    button {
                        class: "text-xs px-2 py-1 rounded bg-gray-700",
                        onclick: move |_| edit_mode.set(false),
                        "Cancel"
                    }
                    button {
                        class: "text-xs px-2 py-1 rounded bg-blue-600",
                        onclick: move |_| {
                            edit_mode.set(false);
                            on_edit_save.call((message_id.clone(), edit_text()));
                        },
                        "Save & regenerate"
                    }
                }
            }
        }
    } else if message.done {
        let bg = if message.role == "user" {
            "bg-blue-700"
        } else {
            "bg-gray-800"
        };
        rsx! {
            div { class: "{align} px-3 py-2 rounded {bg} markdown-body",
                dangerous_inner_html: rendered()
            }
        }
    } else {
        rsx! {
            div { class: "{align} px-3 py-2 rounded bg-gray-800 whitespace-pre-wrap",
                "{message.content}"
                span { class: "animate-pulse", "▍" }
            }
        }
    };

    let has_siblings = message.sibling_count > 1;
    let can_edit = message.role == "user" && message.done && !edit_mode();
    if !has_siblings && !can_edit {
        return bubble;
    }

    let siblings = branches::siblings_of(&history, &message.id);
    let current_index = siblings
        .iter()
        .position(|id| id == &message.id)
        .unwrap_or(message.sibling_index.saturating_sub(1));
    let prev_target = current_index
        .checked_sub(1)
        .and_then(|i| siblings.get(i))
        .cloned();
    let next_target = siblings.get(current_index + 1).cloned();
    rsx! {
        div { class: "space-y-1",
            {bubble}
            div { class: "flex items-center gap-1 {align} text-xs text-gray-400",
                if has_siblings {
                    button {
                        class: if prev_target.is_none() { "opacity-30" } else { "cursor-pointer" },
                        onclick: move |_| {
                            if let Some(target) = prev_target.clone() {
                                on_switch.call(target);
                            }
                        },
                        "‹"
                    }
                    span { "{message.sibling_index}/{message.sibling_count}" }
                    button {
                        class: if next_target.is_none() { "opacity-30" } else { "cursor-pointer" },
                        onclick: move |_| {
                            if let Some(target) = next_target.clone() {
                                on_switch.call(target);
                            }
                        },
                        "›"
                    }
                }
                if can_edit {
                    button {
                        class: "ml-2 cursor-pointer hover:text-gray-200",
                        onclick: move |_| {
                            edit_text.set(message.content.clone());
                            edit_mode.set(true);
                        },
                        "Edit"
                    }
                }
            }
        }
    }
}

fn append_delta(mut messages: Signal<Vec<ChatMessageState>>, message_id: &str, content: &str) {
    let mut list = messages.write();
    if let Some(slot) = list.iter_mut().find(|m| m.id == message_id) {
        slot.content.push_str(content);
    } else {
        list.push(ChatMessageState::plain(
            message_id.to_string(),
            "assistant",
            content.to_string(),
            false,
        ));
    }
}

fn finalize_message(mut messages: Signal<Vec<ChatMessageState>>, message_id: &str, data: &Value) {
    let mut list = messages.write();
    if let Some(slot) = list.iter_mut().find(|m| m.id == message_id) {
        slot.done = true;
        if let Some(content) = data["data"]["content"].as_str() {
            slot.content = content.to_string();
        }
    }
    let _ = data;
}

fn mark_error(mut messages: Signal<Vec<ChatMessageState>>, message_id: &str) {
    let mut list = messages.write();
    if let Some(slot) = list.iter_mut().find(|m| m.id == message_id) {
        slot.is_error = true;
        slot.done = true;
    }
}
