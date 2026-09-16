//! Rusty-Chat Dioxus CSR frontend — M1-6: login (signin/signup), chat list,
//! streaming chat over the native WebSocket.
//!
//! M1 scope notes (see docs/PROGRESS.md M1-6):
//! - messages render as preformatted plain text during streaming; the
//!   comrak+katex-rs+tree-sitter markdown pipeline is the next increment.
//! - one app-level WebSocket; events filter by chat_id.

mod api;

use dioxus::prelude::*;
use serde_json::{json, Value};

#[derive(Clone, Debug)]
struct ChatEntry {
    id: String,
    title: String,
    active: bool,
}

#[derive(Clone, Debug)]
struct ChatMessageState {
    id: String,
    role: String,
    content: String,
    done: bool,
    is_error: bool,
}

fn main() {
    launch(app);
}

fn app() -> Element {
    let main_css = asset!("/assets/main.css");
    let mut token = use_signal(api::token);
    let generation_active = use_signal(|| false);

    rsx! {
        document::Link { rel: "stylesheet", href: main_css }
        if token().is_none() {
            login_view { on_signed_in: move |t| { token.set(Some(t)); } }
        } else {
            div { class: "flex h-screen",
                chat_list_view {
                    token: token().unwrap_or_default(),
                    generation_active: generation_active(),
                    on_sign_out: move |_| {
                        api::clear_token();
                        token.set(None);
                    },
                }
                chat_view {
                    token: token().unwrap_or_default(),
                    generation_active: generation_active,
                }
            }
        }
    }
}

#[allow(dead_code)]
fn unused_original_app() -> Element {
    let mut token = use_signal(api::token);
    let generation_active = use_signal(|| false);

    if token().is_none() {
        rsx! {
            login_view { on_signed_in: move |t| { token.set(Some(t)); } }
        }
    } else {
        rsx! {
            div { class: "flex h-screen",
                chat_list_view {
                    token: token().unwrap_or_default(),
                    generation_active: generation_active(),
                    on_sign_out: move |_| {
                        api::clear_token();
                        token.set(None);
                    },
                }
                chat_view {
                    token: token().unwrap_or_default(),
                    generation_active: generation_active,
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
        to_owned![mode_signup, email, password, name, error, busy, on_signed_in];
        async move {
            busy.set(true);
            error.set(String::new());
            let path = if mode_signup() { "/api/v1/auths/signup" } else { "/api/v1/auths/signin" };
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
    on_sign_out: EventHandler<()>,
) -> Element {
    let chats = use_signal(Vec::<ChatEntry>::new);
    let mut selected_id = use_signal(|| None::<String>);
    let reload_nonce = use_signal(|| 0u32);

    use_effect(move || {
        reload_nonce();
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
        // expose selection to sibling via global-ish signal bus
        div { style: "display:none", {selected_id().map(|i| i).unwrap_or_default()} }
    }
}

#[component]
fn chat_view(token: String, generation_active: Signal<bool>) -> Element {
    let models = use_signal(Vec::<String>::new);
    let mut selected_model = use_signal(String::new);
    let messages = use_signal(Vec::<ChatMessageState>::new);
    let mut input = use_signal(String::new);
    let chat_id = use_signal(|| None::<String>);
    let mut generation_done_nonce = use_signal(|| 0u32);

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
        to_owned![messages, generation_done_nonce];
        spawn(async move {
            let Some(token) = api::token() else { return };
            api::WsChannel::connect(token, move |frame| {
                if frame["event"] != json!("events") {
                    return;
                }
                let data = &frame["data"]["data"];
                let message_id = frame["data"]["message_id"].as_str().unwrap_or_default().to_string();
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
                    _ => {}
                }
            });
        });
    });

    // reload the message list when generation finishes (server persisted)
    use_effect(move || {
        generation_done_nonce();
        to_owned![messages, chat_id];
        spawn(async move {
            if let Some(id) = chat_id() {
                if let Ok(chat) = api::api_get(&format!("/api/v1/chats/{id}")).await {
                    let history = chat["chat"]["history"]["messages"].as_object().cloned().unwrap_or_default();
                    let mut loaded: Vec<ChatMessageState> = history
                        .iter()
                        .filter_map(|(id, m)| {
                            Some(ChatMessageState {
                                id: id.clone(),
                                role: m["role"].as_str().unwrap_or("user").to_string(),
                                content: m["content"].as_str().unwrap_or_default().to_string(),
                                done: m["done"].as_bool().unwrap_or(true),
                                is_error: false,
                            })
                        })
                        .collect();
                    // order by timestamp then id
                    loaded.sort_by_key(|m| m.id.clone());
                    messages.set(loaded);
                }
            }
        });
    });

    let send = move |_| {
        to_owned![input, messages, selected_model, chat_id];
        async move {
            let content = input().trim().to_string();
            if content.is_empty() || selected_model().is_empty() {
                return;
            }
            input.set(String::new());

            let assistant_id = api::uuid_v4();
            let user_message_id = api::uuid_v4();
            let this_chat = chat_id().unwrap_or_else(api::uuid_v4);
            chat_id.set(Some(this_chat.clone()));

            messages.push(ChatMessageState {
                id: user_message_id.clone(),
                role: "user".into(),
                content: content.clone(),
                done: true,
                is_error: false,
            });
            messages.push(ChatMessageState {
                id: assistant_id.clone(),
                role: "assistant".into(),
                content: String::new(),
                done: false,
                is_error: false,
            });

            let body = json!({
                "model": selected_model(),
                "messages": [{"role": "user", "content": content}],
                "stream": true,
                "id": assistant_id,
                "parent_id": Value::Null, // M1: single-turn roots; follow-ups pass the parent id
                "chat_id": this_chat,
                "user_message": {"id": user_message_id, "role": "user", "content": content},
                "session_id": null,
            });
            if let Ok((status, response)) = api::api_post("/api/chat/completions", &body).await {
                if status != 200 {
                    let detail = response["detail"].as_str().unwrap_or("request failed").to_string();
                    mark_error(messages, &assistant_id);
                    let _ = detail;
                    generation_done_nonce += 1;
                }
            }
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
                    div {
                        key: "{message.id}",
                        class: if message.role == "user" { "ml-auto max-w-xl px-3 py-2 rounded bg-blue-700 whitespace-pre-wrap" }
                               else if message.is_error { "max-w-xl px-3 py-2 rounded bg-red-900 whitespace-pre-wrap" }
                               else { "mr-auto max-w-2xl px-3 py-2 rounded bg-gray-800 whitespace-pre-wrap" },
                        "{message.content}"
                        if !message.done { span { class: "animate-pulse", "▍" } }
                    }
                }
            }
            div { class: "p-3 border-t border-gray-800",
                input {
                    class: "w-full px-3 py-2 rounded bg-gray-800",
                    placeholder: if generation_active() { "generating…" } else { "Message Rusty-Chat" },
                    value: input(),
                    oninput: move |e| input.set(e.value()),
                    onkeydown: move |e| {
                        if e.key() == Key::Enter { spawn(send(())); }
                    },
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
        list.push(ChatMessageState {
            id: message_id.to_string(),
            role: "assistant".into(),
            content: content.to_string(),
            done: false,
            is_error: false,
        });
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
