//! Chat view — open-webui layout imitation: navbar with chat title, centered
//! placeholder (model name + suggestions + input) for the empty state, and
//! the message list with right-aligned user bubbles / assistant blocks with
//! avatar + model name, plus the bottom input bar with a model selector.
//!
//! 覆盖矩阵（tests below）:
//! ✅ build_view 投影（active_path + 兄弟位置 + model 字段透传）
//! ✅ 消息组件 key 稳定性（内容变化才重建 memo 渲染）
//! ✅ 流式 append/finalize/error 三条路径（slot 缺失时补建）
//! ⛔ 刻意不覆盖：WS 流式/发送/编辑分支的端到端行为（native contract 测试
//!    与浏览器冒烟覆盖，对应 contract_branches / contract_tasks）；
//!    DOM 滚动副作用（浏览器冒烟）。

use dioxus::prelude::*;
use serde_json::{Value, json};
use wasm_bindgen::JsCast;

use crate::api;
use crate::branches;
use crate::icons;
use crate::render;

/// open-webui's built-in default prompt suggestions (config.py
/// DEFAULT_PROMPT_SUGGESTIONS): title shown bold, subtitle below. A `static`
/// so the &'static strs captured by click closures are genuinely 'static.
static SUGGESTIONS: [(&str, &str); 6] = [
    ("Help me study", "vocabulary for a college entrance exam"),
    ("Give me ideas", "for what to do with my kids' art"),
    ("Tell me a fun fact", "about the Roman Empire"),
    ("Show me a code snippet", "of a website's sticky header"),
    (
        "Explain options trading",
        "if I'm familiar with buying and selling stocks",
    ),
    ("Overcome procrastination", "give me tips"),
];

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ChatMessageState {
    id: String,
    role: String,
    content: String,
    done: bool,
    is_error: bool,
    /// model id recorded on the assistant node (displayed above the body)
    model: Option<String>,
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
            model: None,
            sibling_index: 0,
            sibling_count: 0,
        }
    }
}

#[component]
pub(crate) fn chat_view(
    generation_active: Signal<bool>,
    mut list_refresh: Signal<u32>,
    mut selected: Signal<Option<String>>,
) -> Element {
    let models = use_signal(Vec::<String>::new);
    let selected_model = use_signal(String::new);
    let mut messages = use_signal(Vec::<ChatMessageState>::new);
    let mut history = use_signal(Value::default);
    let input = use_signal(String::new);
    let mut chat_id = use_signal(|| None::<String>);
    let mut generation_done_nonce = use_signal(|| 0u32);
    let mut title = use_signal(|| "新对话".to_string());
    let mut at_bottom = use_signal(|| true);

    // sidebar selection drives the open chat; None starts a new chat
    use_effect(move || {
        let picked = selected();
        if picked != chat_id() {
            let starts_new_chat = picked.is_none();
            chat_id.set(picked);
            messages.set(Vec::new());
            history.set(Value::default());
            if starts_new_chat {
                title.set("新对话".to_string());
            }
            at_bottom.set(true);
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
        to_owned![
            messages,
            generation_done_nonce,
            generation_active,
            list_refresh
        ];
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
                        append_delta(&mut messages.write(), &message_id, content);
                    }
                    "chat:message" => {
                        if data["data"].get("done") == Some(&json!(true)) {
                            finalize_message(&mut messages.write(), &message_id, data);
                            generation_done_nonce += 1;
                        }
                    }
                    "chat:message:error" => {
                        mark_error(&mut messages.write(), &message_id);
                        generation_done_nonce += 1;
                    }
                    "chat:active" => {
                        let active = data["data"]["active"].as_bool().unwrap_or(false);
                        generation_active.set(active);
                    }
                    "chat:title" => {
                        // sidebar reloads and picks up the new title; the
                        // navbar title follows via the reload effect below
                        list_refresh += 1;
                    }
                    _ => {}
                }
            });
        });
    });

    // reload the message list when generation finishes or the chat list
    // changes (server persisted; the title column may have been generated in
    // the background); display follows the active branch path of
    // history.currentId
    use_effect(move || {
        generation_done_nonce();
        list_refresh();
        to_owned![messages, chat_id, history, title];
        spawn(async move {
            if let Some(id) = chat_id()
                && let Ok(chat) = api::api_get(&format!("/api/v1/chats/{id}")).await
            {
                title.set(
                    chat["title"]
                        .as_str()
                        .filter(|t| !t.is_empty())
                        .unwrap_or("新对话")
                        .to_string(),
                );
                let blob_history = chat["chat"]["history"].clone();
                history.set(blob_history.clone());
                messages.set(build_view(&blob_history));
            }
        });
    });

    // follow the stream while pinned to the bottom (deltas append content,
    // which touches the messages signal and re-fires this effect). A delayed
    // re-pin covers content that grows after the effect ran (async mermaid /
    // highlight backfill on chat open).
    use_effect(move || {
        messages();
        if at_bottom() {
            scroll_messages_to_bottom();
            to_owned![at_bottom];
            spawn(async move {
                gloo_timers::future::TimeoutFuture::new(300).await;
                if at_bottom() {
                    scroll_messages_to_bottom();
                }
            });
        }
    });

    let view: Vec<(String, ChatMessageState)> = messages()
        .iter()
        .map(|m| (message_key(m), m.clone()))
        .collect();
    let message_count = view.len();
    let is_empty = message_count == 0;

    rsx! {
        main { class: "flex-1 flex flex-col h-screen min-w-0",
            chat_navbar { title: title() }

            div {
                id: "messages-container",
                class: "flex-1 w-full overflow-y-auto scrollbar-hidden",
                onscroll: move |_| {
                    if let Some(el) = messages_container() {
                        // 40px window: small enough that barely-overflowing
                        // content (short chats) is not permanently pinned to
                        // the bottom
                        at_bottom.set(el.scroll_top() + el.client_height() >= el.scroll_height() - 40);
                    }
                },
                if is_empty {
                    placeholder_view {
                        models,
                        selected_model,
                        input,
                        on_send: move |()| submit(
                            input, history, messages, selected_model, chat_id,
                            selected, generation_done_nonce, generation_active,
                        ),
                    }
                } else {
                    div { class: "w-full pt-2 pb-24",
                        for (index, (key, message)) in view.into_iter().enumerate() {
                            message_item {
                                key: "{key}",
                                message,
                                is_last: index + 1 == message_count,
                                streaming_model: selected_model(),
                                history: history(),
                                on_switch: move |target| switch_branch(
                                    chat_id, history, generation_done_nonce, target,
                                ),
                                on_edit_save: move |(old_id, content)| save_edit(
                                    history, chat_id, selected_model, messages,
                                    generation_done_nonce, old_id, content,
                                ),
                            }
                        }
                    }
                }
            }

            if !is_empty {
                div { class: "relative shrink-0 w-full px-2 pb-2",
                    if !at_bottom() {
                        div { class: "absolute -top-12 left-0 right-0 z-30 flex justify-center pointer-events-none",
                            button {
                                class: "pointer-events-auto p-1.5 rounded-full bg-white/20 hover:bg-white/30 text-gray-100 transition cursor-pointer",
                                title: "滚动到底部",
                                onclick: move |_| {
                                    at_bottom.set(true);
                                    scroll_messages_to_bottom();
                                },
                                icons::ArrowUp { class: "size-5 rotate-180" }
                            }
                        }
                    }
                    div { class: "max-w-[58rem] mx-auto",
                        message_input {
                            input,
                            disabled: generation_active(),
                            placeholder: "输入消息",
                            models: models(),
                            selected_model,
                            on_send: move |()| submit(
                                input, history, messages, selected_model, chat_id,
                                selected, generation_done_nonce, generation_active,
                            ),
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn chat_navbar(title: String) -> Element {
    rsx! {
        nav { class: "flex items-center w-full px-3 pt-2 pb-1 shrink-0 z-30",
            div { class: "flex-1 min-w-0 pl-1",
                div { class: "truncate py-1 text-left text-[0.9375rem] font-normal text-gray-300",
                    "{title}"
                }
            }
        }
    }
}

/// Empty chat: centered model name, the input box, and open-webui's default
/// suggestion cards.
#[component]
fn placeholder_view(
    models: Signal<Vec<String>>,
    mut selected_model: Signal<String>,
    mut input: Signal<String>,
    on_send: EventHandler<()>,
) -> Element {
    let avatar_char = selected_model
        .read()
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "AI".to_string());
    rsx! {
        div { class: "h-full flex flex-col items-center justify-center px-4",
            div { class: "w-full max-w-[58rem] text-center",
                div { class: "flex items-center justify-center gap-3",
                    div { class: "flex size-10 shrink-0 items-center justify-center rounded-2xl bg-gradient-to-br from-blue-500 to-blue-700 text-lg font-bold text-white",
                        "{avatar_char}"
                    }
                    button {
                        class: "text-2xl text-gray-100 max-w-[24rem] truncate hover:opacity-80 transition cursor-pointer",
                        title: "点击切换模型",
                        onclick: move |_| {
                            let list = models.read().clone();
                            let current = selected_model.read().clone();
                            if list.len() > 1 {
                                let next = list
                                    .iter()
                                    .position(|m| m == &current)
                                    .map(|i| (i + 1) % list.len())
                                    .unwrap_or(0);
                                selected_model.set(list[next].clone());
                            }
                        },
                        if selected_model.read().is_empty() { "Rusty-Chat" } else { "{selected_model}" }
                    }
                }
                div { class: "mt-6",
                    message_input {
                        input,
                        disabled: false,
                        placeholder: "有什么我能帮您的吗？",
                        models: models(),
                        selected_model,
                        on_send,
                    }
                }
                div { class: "mx-auto max-w-2xl mt-3 text-left",
                    div { class: "mb-1 flex items-center gap-1 text-xs font-normal text-gray-500",
                        icons::Zap { class: "size-3.5" }
                        "建议"
                    }
                    div { class: "flex flex-col",
                        for (index, (prompt_title, prompt_subtitle)) in SUGGESTIONS.iter().enumerate() {
                            button {
                                key: "{prompt_title}",
                                class: "waterfall group flex flex-col px-2.5 py-1.5 rounded-lg hover:bg-white/[0.04] transition-colors text-left cursor-pointer",
                                style: format!("animation-delay: {}ms", index * 45),
                                onclick: move |_| {
                                    input.set(format!("{prompt_title} {prompt_subtitle}"));
                                },
                                span { class: "text-sm font-normal text-gray-300 group-hover:text-white transition line-clamp-1",
                                    "{prompt_title}"
                                }
                                span { class: "text-xs font-normal text-gray-500 group-hover:text-gray-300 transition line-clamp-1",
                                    "{prompt_subtitle}"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The open-webui message input shell: rounded-3xl bordered card, autogrow
/// textarea on top, attachment placeholder left / model selector + send on
/// the right.
#[component]
fn message_input(
    mut input: Signal<String>,
    disabled: bool,
    placeholder: &'static str,
    models: Vec<String>,
    selected_model: Signal<String>,
    on_send: EventHandler<()>,
) -> Element {
    let mut menu_open = use_signal(|| false);
    let rows = (input().lines().count().clamp(1, 10)) as u32;
    let active = !input().trim().is_empty() && !disabled;

    rsx! {
        div { class: "relative w-full text-left",
            if menu_open() {
                button {
                    class: "fixed inset-0 z-40 cursor-default",
                    tabindex: -1,
                    onclick: move |_| menu_open.set(false),
                }
                div { class: "absolute bottom-12 right-0 z-50 w-[20rem] max-w-[calc(100vw-2rem)] rounded-xl border border-gray-800 bg-gray-850 p-1 shadow-xl",
                    for model in models.iter().cloned() {
                        button {
                            key: "{model}",
                            class: "flex w-full items-center justify-between gap-2 rounded-lg px-2.5 py-1.5 text-[0.8125rem] hover:bg-gray-900 transition cursor-pointer text-left",
                            onclick: move |_| {
                                selected_model.set(model.clone());
                                menu_open.set(false);
                            },
                            span { class: "truncate text-gray-200", "{model}" }
                            if selected_model() == model {
                                span { class: "text-gray-400", "✓" }
                            }
                        }
                    }
                    if models.is_empty() {
                        div { class: "px-2.5 py-2 text-xs text-gray-500", "没有可用模型" }
                    }
                }
            }

            div { class: "flex flex-col rounded-3xl border border-gray-850 bg-gray-500/5 shadow-lg px-2 pt-1.5 pb-1 transition hover:border-gray-800 focus-within:border-gray-800",
                textarea {
                    class: "w-full bg-transparent outline-none resize-none text-[0.9375rem] text-gray-100 placeholder-gray-500 px-2 leading-6 max-h-48 overflow-y-auto scrollbar-none",
                    placeholder: "{placeholder}",
                    value: input(),
                    rows,
                    oninput: move |e| input.set(e.value()),
                    onkeydown: move |e| {
                        // Shift+Enter is a newline; IME confirm (composition)
                        // must not send
                        if e.key() == Key::Enter
                            && !e.modifiers().contains(Modifiers::SHIFT)
                            && !e.is_composing()
                        {
                            e.prevent_default();
                            on_send.call(());
                        }
                    },
                }
                div { class: "flex items-center justify-between mt-1 mb-0.5 mx-0.5",
                    div { class: "flex items-center",
                        button {
                            class: "flex size-[1.875rem] items-center justify-center rounded-full text-gray-400 opacity-50 cursor-default",
                            title: "附件（即将支持）",
                            disabled: true,
                            icons::Plus { class: "size-5" }
                        }
                        div { class: "w-px h-4 mx-2 bg-gray-800/70" }
                    }
                    div { class: "flex items-center gap-1",
                        button {
                            class: "flex items-center gap-1.5 rounded-lg pl-2 pr-1.5 py-1 text-[0.8125rem] font-normal text-gray-300 hover:bg-gray-800/40 transition max-w-[13rem] cursor-pointer",
                            onclick: move |_| menu_open.toggle(),
                            span { class: "truncate min-w-0",
                                if selected_model().is_empty() { "选择模型" } else { "{selected_model}" }
                            }
                            icons::ChevronDown { class: "size-2.5 shrink-0" }
                        }
                        button {
                            class: if active {
                                "flex size-[1.875rem] items-center justify-center rounded-full bg-white text-gray-900 hover:bg-gray-200 transition cursor-pointer"
                            } else {
                                "flex size-[1.875rem] items-center justify-center rounded-full bg-gray-700 text-gray-500 transition cursor-default"
                            },
                            disabled: !active,
                            onclick: move |_| on_send.call(()),
                            title: "发送",
                            icons::ArrowUp { class: "size-5" }
                        }
                    }
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
                model: m["model"].as_str().map(str::to_string),
                sibling_index,
                sibling_count,
            }
        })
        .collect()
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

/// Follow-up send (open-webui submitPrompt semantics): a follow-up parents
/// onto history.currentId, the LLM receives the full active chain, the local
/// tree is patched (mirroring the server upsert) and the view re-projected —
/// never a flat list push.
#[allow(clippy::too_many_arguments)]
fn submit(
    mut input: Signal<String>,
    mut history: Signal<Value>,
    mut messages: Signal<Vec<ChatMessageState>>,
    selected_model: Signal<String>,
    mut chat_id: Signal<Option<String>>,
    mut selected: Signal<Option<String>>,
    mut generation_done_nonce: Signal<u32>,
    generation_active: Signal<bool>,
) {
    let content = input().trim().to_string();
    if content.is_empty() || selected_model().is_empty() || generation_active() {
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

    let parent: Option<String> = history()
        .get("currentId")
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty())
        .map(str::to_string);
    let chain = branches::messages_for_regeneration(&history(), parent.as_deref(), &content);

    let mut local = history();
    let now = api::epoch_secs();
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
        &selected_model(),
    );
    history.set(local.clone());
    messages.set(build_view(&local));
    scroll_messages_to_bottom();

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
    spawn(async move {
        if let Ok((status, response)) = api::api_post("/api/chat/completions", &body).await
            && status != 200
        {
            let _ = response;
            mark_error(&mut messages.write(), &assistant_id);
            generation_done_nonce += 1;
        }
    });
}

/// Switch to a sibling branch: currentId lands on the sibling's LEAF
/// (open-webui semantics) so the whole branch shows, then reload.
fn switch_branch(
    chat_id: Signal<Option<String>>,
    history: Signal<Value>,
    mut generation_done_nonce: Signal<u32>,
    target_id: String,
) {
    spawn(async move {
        let Some(this_chat) = chat_id() else { return };
        let leaf = branches::leaf_descendant(&history(), &target_id);
        let _ = api::api_post(
            &format!("/api/v1/chats/{this_chat}"),
            &json!({"chat": {"history": {"currentId": leaf}}}),
        )
        .await;
        generation_done_nonce += 1;
    });
}

/// Edit a user message → sibling branch with a fresh assistant response.
/// open-webui editMessage semantics: patch the tree LOCALLY (new sibling
/// under the old message's parent, currentId moves to the new branch) and
/// re-project the view from the active path — so the edited message
/// REPLACES the old one in place instead of appearing at the bottom.
#[allow(clippy::too_many_arguments)]
fn save_edit(
    mut history: Signal<Value>,
    chat_id: Signal<Option<String>>,
    selected_model: Signal<String>,
    mut messages: Signal<Vec<ChatMessageState>>,
    mut generation_done_nonce: Signal<u32>,
    old_user_id: String,
    content: String,
) {
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
        let chain = branches::messages_for_regeneration(&history(), parent.as_deref(), &content);
        let new_user_id = api::uuid_v4();
        let new_assistant_id = api::uuid_v4();

        let mut local = history();
        let now = api::epoch_secs();
        branches::attach_user_message(&mut local, parent.as_deref(), &new_user_id, &content, now);
        branches::attach_assistant_placeholder(
            &mut local,
            &new_user_id,
            &new_assistant_id,
            now,
            &selected_model(),
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

#[component]
fn message_item(
    message: ChatMessageState,
    is_last: bool,
    streaming_model: String,
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
            crate::mermaid::render_mermaid_blocks().await;
            crate::highlight::backfill_code_blocks().await;
        });
    });

    // local edit state (user messages only)
    let mut edit_mode = use_signal(|| false);
    let mut edit_text = use_signal(String::new);

    let is_user = message.role == "user";
    let has_siblings = message.sibling_count > 1;
    let can_edit = is_user && message.done && !edit_mode();

    // owned clones for the event-handler closures (ChatMessageState itself is
    // not Copy and each closure needs its own capture)
    let content_for_copy = message.content.clone();
    let content_for_edit = message.content.clone();

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
    let prev_disabled = if prev_target.is_none() {
        "opacity-30"
    } else {
        ""
    };
    let next_disabled = if next_target.is_none() {
        "opacity-30"
    } else {
        ""
    };

    // assistant header: model recorded on the message, the streaming model as
    // a fallback while its own node hasn't reloaded yet
    let model_name = message
        .model
        .clone()
        .or_else(|| (!message.done).then(|| streaming_model.clone()))
        .unwrap_or_else(|| "助手".to_string());
    let avatar_char = model_name
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "AI".to_string());

    let action_button =
        "p-1.5 rounded-lg hover:bg-white/5 hover:text-gray-200 transition cursor-pointer";

    let body = if message.is_error {
        rsx! {
            div { class: "mt-1 text-sm text-red-400",
                if message.content.is_empty() { "生成失败" } else { "{message.content}" }
            }
        }
    } else if edit_mode() {
        let message_id = message.id.clone();
        let message_id_for_keys = message_id.clone();
        let edit_rows = (edit_text().lines().count().clamp(2, 12)) as u32;
        rsx! {
            div { class: "w-full bg-gray-800 rounded-3xl px-4 py-3 mb-1",
                textarea {
                    class: "w-full bg-transparent outline-none resize-none text-[0.9375rem] text-gray-100 leading-6",
                    value: edit_text(),
                    rows: edit_rows,
                    autofocus: true,
                    oninput: move |e| edit_text.set(e.value()),
                    onkeydown: move |e| {
                        // Ctrl+Enter saves from inside the edit box
                        if e.key() == Key::Enter
                            && e.modifiers().contains(Modifiers::CONTROL)
                            && !e.is_composing()
                        {
                            on_edit_save.call((message_id_for_keys.clone(), edit_text()));
                        }
                    },
                }
                div { class: "mt-2 flex justify-between text-sm",
                    button {
                        class: "px-2.5 py-1 bg-gray-900 hover:bg-gray-800 text-gray-200 rounded-3xl transition cursor-pointer",
                        onclick: move |_| edit_mode.set(false),
                        "取消"
                    }
                    button {
                        class: "px-2.5 py-1 bg-white hover:bg-gray-200 text-gray-900 rounded-3xl transition cursor-pointer",
                        onclick: move |_| {
                            edit_mode.set(false);
                            on_edit_save.call((message_id.clone(), edit_text()));
                        },
                        "发送"
                    }
                }
            }
        }
    } else if message.done {
        if is_user {
            rsx! {
                div { class: "flex w-full justify-end pb-1",
                    div { class: "rounded-3xl max-w-[90%] px-4 py-1.5 bg-gray-850",
                        div { class: "markdown-body", dangerous_inner_html: rendered() }
                    }
                }
            }
        } else {
            rsx! {
                div { class: "markdown-body mt-0.5", dangerous_inner_html: rendered() }
            }
        }
    } else {
        // streaming: plain text + pulsing caret (markdown lands on done)
        rsx! {
            div { class: "mt-1 whitespace-pre-wrap text-[0.9375rem] leading-relaxed text-gray-100",
                "{message.content}"
                span { class: "streaming-caret" }
            }
        }
    };

    let switcher = has_siblings.then(|| {
        rsx! {
            div { class: "flex items-center gap-0.5",
                button {
                    class: "p-1 rounded-md hover:bg-white/5 hover:text-gray-200 transition text-gray-500 cursor-pointer {prev_disabled}",
                    title: "上一个分支",
                    onclick: move |_| {
                        if let Some(target) = prev_target.clone() {
                            on_switch.call(target);
                        }
                    },
                    icons::ChevronLeft { class: "size-3.5" }
                }
                span { class: "text-sm tracking-widest font-normal text-gray-200 min-w-fit",
                    "{message.sibling_index}/{message.sibling_count}"
                }
                button {
                    class: "p-1 rounded-md hover:bg-white/5 hover:text-gray-200 transition text-gray-500 cursor-pointer {next_disabled}",
                    title: "下一个分支",
                    onclick: move |_| {
                        if let Some(target) = next_target.clone() {
                            on_switch.call(target);
                        }
                    },
                    icons::ChevronRight { class: "size-3.5" }
                }
            }
        }
    });

    if is_user {
        rsx! {
            div { class: "flex flex-col px-3.5 mb-3 w-full max-w-[58rem] mx-auto group",
                {body}
                div { class: "flex justify-end items-center gap-1 mt-0.5 text-gray-500",
                    {switcher}
                    if !edit_mode() {
                        button {
                            class: "{action_button} hover-reveal",
                            title: "复制",
                            onclick: move |_| api::copy_text(&content_for_copy),
                            icons::Copy { class: "size-4" }
                        }
                        if can_edit {
                            button {
                                class: "{action_button} hover-reveal",
                                title: "编辑",
                                onclick: move |_| {
                                    edit_text.set(content_for_edit.clone());
                                    edit_mode.set(true);
                                },
                                icons::Pencil { class: "size-4" }
                            }
                        }
                    }
                }
            }
        }
    } else {
        // last message keeps its action row visible, earlier ones reveal on
        // hover (open-webui: isLastMessage ? visible : hover-reveal)
        let actions_reveal = if is_last { "" } else { "hover-reveal" };
        rsx! {
            div { class: "flex flex-col px-3.5 mb-3 w-full max-w-[58rem] mx-auto group",
                div { class: "flex w-full gap-2",
                    div { class: "shrink-0 mt-0.5",
                        div { class: "flex size-7 items-center justify-center rounded-xl bg-gradient-to-br from-gray-600 to-gray-800 text-xs font-semibold text-gray-200",
                            "{avatar_char}"
                        }
                    }
                    div { class: "flex-auto w-0 min-w-0 pl-1",
                        div { class: "text-[0.9375rem] font-normal text-gray-100 line-clamp-1",
                            "{model_name}"
                        }
                        {body}
                        div { class: "flex items-center gap-1 mt-0.5 text-gray-500",
                            {switcher}
                            if !message.is_error && message.done {
                                button {
                                    class: "{action_button} {actions_reveal}",
                                    title: "复制",
                                    onclick: move |_| api::copy_text(&content_for_copy),
                                    icons::Copy { class: "size-4" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn messages_container() -> Option<web_sys::HtmlElement> {
    let window = web_sys::window()?;
    let document = window.document()?;
    document
        .get_element_by_id("messages-container")?
        .dyn_into::<web_sys::HtmlElement>()
        .ok()
}

fn scroll_messages_to_bottom() {
    if let Some(el) = messages_container() {
        el.set_scroll_top(el.scroll_height());
    }
}

fn append_delta(messages: &mut Vec<ChatMessageState>, message_id: &str, content: &str) {
    if let Some(slot) = messages.iter_mut().find(|m| m.id == message_id) {
        slot.content.push_str(content);
    } else {
        messages.push(ChatMessageState::plain(
            message_id.to_string(),
            "assistant",
            content.to_string(),
            false,
        ));
    }
}

fn finalize_message(messages: &mut [ChatMessageState], message_id: &str, data: &Value) {
    if let Some(slot) = messages.iter_mut().find(|m| m.id == message_id) {
        slot.done = true;
        if let Some(content) = data["data"]["content"].as_str() {
            slot.content = content.to_string();
        }
    }
}

fn mark_error(messages: &mut [ChatMessageState], message_id: &str) {
    if let Some(slot) = messages.iter_mut().find(|m| m.id == message_id) {
        slot.is_error = true;
        slot.done = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// straight-line chain projects in order with model passthrough
    #[test]
    fn build_view_linear_chain_with_model() {
        let history = json!({
            "currentId": "a2",
            "messages": {
                "u1": {"id": "u1", "parentId": null, "childrenIds": ["a1"], "role": "user", "content": "hi", "done": true},
                "a1": {"id": "a1", "parentId": "u1", "childrenIds": ["u2"], "role": "assistant", "content": "hello", "done": true, "model": "m1"},
                "u2": {"id": "u2", "parentId": "a1", "childrenIds": ["a2"], "role": "user", "content": "more", "done": true},
                "a2": {"id": "a2", "parentId": "u2", "childrenIds": [], "role": "assistant", "content": "done", "done": true, "model": "m2"},
            }
        });
        let view = build_view(&history);
        let ids: Vec<&str> = view.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["u1", "a1", "u2", "a2"]);
        assert_eq!(view[1].model.as_deref(), Some("m1"));
        assert!(
            view.iter()
                .all(|m| m.sibling_count == 1 && m.sibling_index == 1)
        );
    }

    /// branch point shows sibling positions; the inactive branch is absent
    #[test]
    fn build_view_sibling_positions() {
        let history = json!({
            "currentId": "a2",
            "messages": {
                "u1": {"id": "u1", "parentId": null, "childrenIds": ["a1", "a2"], "role": "user", "content": "q", "done": true},
                "a1": {"id": "a1", "parentId": "u1", "childrenIds": [], "role": "assistant", "content": "old", "done": true},
                "a2": {"id": "a2", "parentId": "u1", "childrenIds": [], "role": "assistant", "content": "new", "done": true},
            }
        });
        let view = build_view(&history);
        assert_eq!(view.len(), 2);
        assert_eq!(view[1].id, "a2");
        assert_eq!(view[1].sibling_index, 2);
        assert_eq!(view[1].sibling_count, 2);
    }

    /// the message key changes only when id, done flag, or content change
    #[test]
    fn message_key_stability() {
        let base = ChatMessageState::plain("m".into(), "assistant", "abc".into(), false);
        let same = ChatMessageState::plain("m".into(), "assistant", "abc".into(), false);
        let mut streamed = base.clone();
        streamed.content = "abcd".into();
        let mut finished = base.clone();
        finished.done = true;

        assert_eq!(message_key(&base), message_key(&same));
        assert_ne!(message_key(&base), message_key(&streamed));
        assert_ne!(message_key(&base), message_key(&finished));
    }

    /// streaming deltas append into the placeholder slot; a missing slot
    /// creates one
    #[test]
    fn delta_append_and_slot_creation() {
        let mut messages = vec![ChatMessageState::plain(
            "a1".into(),
            "assistant",
            String::new(),
            false,
        )];
        append_delta(&mut messages, "a1", "Hel");
        append_delta(&mut messages, "a1", "lo");
        assert_eq!(messages[0].content, "Hello");

        append_delta(&mut messages, "ghost", "x");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].id, "ghost");
    }

    /// finalize flips done and replaces content; error marks are terminal
    #[test]
    fn finalize_and_error_paths() {
        let mut messages = vec![ChatMessageState::plain(
            "a1".into(),
            "assistant",
            "partial".into(),
            false,
        )];
        finalize_message(&mut messages, "a1", &json!({"data": {"content": "final"}}));
        assert!(messages[0].done);
        assert_eq!(messages[0].content, "final");

        mark_error(&mut messages, "a1");
        assert!(messages[0].is_error);
        assert!(messages[0].done);

        // error for an unknown message id is a no-op (no phantom slot)
        mark_error(&mut messages, "ghost");
        assert_eq!(messages.len(), 1);
    }
}
