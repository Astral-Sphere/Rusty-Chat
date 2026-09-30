//! Rusty-Chat Dioxus CSR frontend — open-webui-style shell: login, sidebar
//! (chat list / search / user menu), and the chat view. Message rendering
//! pipeline lives in render.rs + highlight.rs + mermaid.rs; tree semantics in
//! branches.rs; view logic in chat.rs + sidebar.rs.

mod api;
mod branches;
mod chat;
mod highlight;
mod icons;
mod mermaid;
mod render;
mod sidebar;

use dioxus::prelude::*;
use serde_json::json;

fn main() {
    launch(app);
}

fn app() -> Element {
    // Generated from web/input.css by the Tailwind v4 standalone CLI
    // (`just web-css`; gitignored, regenerate once after a fresh clone).
    let main_css = asset!("/assets/tailwind.css");
    let katex_css = asset!("/assets/katex/katex.min.css");
    let mut token = use_signal(api::token);
    let mut sidebar_open = use_signal(|| true);
    let generation_active = use_signal(|| false);
    // bumped when a background task changes chat state (title generation)
    let list_refresh = use_signal(|| 0u32);
    let selected_chat = use_signal(|| None::<String>);
    let user_name = use_signal(String::new);

    // session user for the sidebar footer, fetched once per sign-in
    use_effect(move || {
        if token().is_none() {
            return;
        }
        to_owned![user_name];
        spawn(async move {
            if let Ok(user) = api::api_get("/api/v1/auths/").await {
                user_name.set(user["name"].as_str().unwrap_or_default().to_string());
            }
        });
    });

    rsx! {
        document::Link { rel: "stylesheet", href: main_css }
        document::Link { rel: "stylesheet", href: katex_css }
        if token().is_none() {
            login_view { on_signed_in: move |t| { token.set(Some(t)); } }
        } else {
            div { class: "flex h-screen",
                sidebar::sidebar {
                    open: sidebar_open(),
                    user_name: user_name(),
                    refresh: list_refresh,
                    selected: selected_chat,
                    on_toggle: move |_| sidebar_open.toggle(),
                    on_sign_out: move |_| {
                        api::clear_token();
                        token.set(None);
                    },
                }
                chat::chat_view {
                    generation_active,
                    list_refresh,
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

    let field = "w-full rounded-xl border border-gray-800 bg-gray-900 px-3 py-2 text-sm text-gray-100 placeholder-gray-500 outline-none focus:border-gray-600 transition";
    let tab = |active: bool| {
        if active {
            "flex-1 rounded-full py-1.5 text-sm bg-gray-800 text-gray-100 transition cursor-pointer"
        } else {
            "flex-1 rounded-full py-1.5 text-sm text-gray-500 hover:text-gray-300 transition cursor-pointer"
        }
    };

    rsx! {
        div { class: "min-h-screen flex items-center justify-center bg-[#171717] text-gray-100",
            div { class: "w-96 flex flex-col",
                div { class: "flex flex-col items-center mb-6",
                    span { class: "flex size-10 items-center justify-center rounded-2xl bg-gradient-to-br from-blue-500 to-blue-700 text-lg font-bold text-white mb-3",
                        "R"
                    }
                    h1 { class: "text-2xl font-semibold", "Rusty-Chat" }
                }
                div { class: "flex rounded-full bg-gray-900 p-1 mb-4",
                    button {
                        class: tab(!mode_signup()),
                        onclick: move |_| mode_signup.set(false),
                        "登录"
                    }
                    button {
                        class: tab(mode_signup()),
                        onclick: move |_| mode_signup.set(true),
                        "注册"
                    }
                }
                if mode_signup() {
                    input {
                        class: "{field} mb-2",
                        placeholder: "姓名",
                        value: name(),
                        oninput: move |e| name.set(e.value()),
                    }
                }
                input {
                    class: "{field} mb-2",
                    placeholder: "邮箱",
                    value: email(),
                    oninput: move |e| email.set(e.value()),
                }
                input {
                    class: "{field} mb-2",
                    placeholder: "密码",
                    r#type: "password",
                    value: password(),
                    oninput: move |e| password.set(e.value()),
                }
                if !error().is_empty() {
                    div { class: "text-red-400 text-sm mb-2", "{error}" }
                }
                button {
                    class: "w-full rounded-xl bg-white py-2 text-sm font-medium text-gray-900 hover:bg-gray-200 transition disabled:opacity-50 cursor-pointer",
                    disabled: busy(),
                    onclick: submit,
                    if busy() { "请稍候…" } else if mode_signup() { "创建账号" } else { "登录" }
                }
            }
        }
    }
}
