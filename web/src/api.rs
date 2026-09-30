//! Browser API helpers: localStorage token persistence, HTTP via gloo-net,
//! a reconnecting WebSocket channel, and a small uuid generator.

use serde_json::{Value, json};
use wasm_bindgen::JsCast;
use web_sys::window;

pub fn api_base() -> String {
    // Same-origin API; dx dev proxy forwards to the rusty-chat server.
    window().unwrap().location().origin().unwrap()
}

pub fn token() -> Option<String> {
    window()
        .unwrap()
        .local_storage()
        .ok()
        .flatten()
        .and_then(|s| s.get_item("token").ok().flatten())
}

pub fn set_token(token: &str) {
    if let Some(storage) = window().unwrap().local_storage().ok().flatten() {
        storage.set_item("token", token).ok();
    }
}

pub fn clear_token() {
    if let Some(storage) = window().unwrap().local_storage().ok().flatten() {
        storage.delete("token").ok();
    }
}

use gloo_net::http::{Request, RequestBuilder};

fn auth_headers(builder: RequestBuilder) -> RequestBuilder {
    if let Some(token) = token() {
        return builder.header("Authorization", &format!("Bearer {token}"));
    }
    builder
}

/// GET returning parsed JSON.
pub async fn api_get(path: &str) -> Result<Value, String> {
    auth_headers(Request::get(&format!("{}{path}", api_base())))
        .build()
        .map_err(|e| e.to_string())?
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json::<Value>()
        .await
        .map_err(|e| e.to_string())
}

/// POST a JSON body returning (status, json).
pub async fn api_post(path: &str, body: &Value) -> Result<(u16, Value), String> {
    let request = auth_headers(Request::post(&format!("{}{path}", api_base())))
        .json(body)
        .map_err(|e| e.to_string())?;
    let response = request.send().await.map_err(|e| e.to_string())?;
    let status = response.status();
    let body = response.json::<Value>().await.unwrap_or(Value::Null);
    Ok((status, body))
}

/// Reserved for M1-7 chat deletion from the sidebar UI.
#[allow(dead_code)]
pub async fn api_delete(path: &str) -> Result<(u16, Value), String> {
    let request = auth_headers(Request::delete(&format!("{}{path}", api_base())))
        .build()
        .map_err(|e| e.to_string())?;
    let response = request.send().await.map_err(|e| e.to_string())?;
    let status = response.status();
    let body = response.json::<Value>().await.unwrap_or(Value::Null);
    Ok((status, body))
}

/// Pseudo-uuid (crypto.getRandomValues) — ids are client-side only.
pub fn uuid_v4() -> String {
    let mut bytes = [0u8; 16];
    for b in bytes.iter_mut() {
        *b = (js_sys::Math::random() * 256.0) as u8;
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// Thin wrapper over the browser WebSocket delivering text frames as JSON.
pub struct WsChannel {
    /// held so the socket stays open; frames flow through the message closure
    #[allow(dead_code)]
    ws: web_sys::WebSocket,
}

impl WsChannel {
    /// Connects to `/ws` and performs the token handshake. `on_event` fires
    /// for every `{"event": "events", "data": …}` frame (and connected).
    pub fn connect(token: String, mut on_message: impl FnMut(Value) + 'static) -> Self {
        let origin = window().unwrap().location().origin().unwrap();
        let ws_origin = origin.replacen("http", "ws", 1);
        let ws = web_sys::WebSocket::new(&format!("{ws_origin}/ws")).unwrap();

        // handshake: send the token once the socket opens
        let ws_for_open = ws.clone();
        let handshake_token = token.clone();
        let onopen = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(
            move |_event: web_sys::Event| {
                ws_for_open
                    .send_with_str(&json!({"token": handshake_token}).to_string())
                    .ok();
            },
        );
        ws.set_onopen(Some(onopen.as_ref().unchecked_ref()));
        onopen.forget(); // socket lives for the app session

        let onmessage = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::MessageEvent)>::new(
            move |event: web_sys::MessageEvent| {
                if let Some(text) = event.data().as_string()
                    && let Ok(value) = serde_json::from_str::<Value>(&text)
                {
                    on_message(value);
                }
            },
        );
        ws.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
        onmessage.forget();

        Self { ws }
    }

    /// Client→server frames (heartbeats); reserved for presence features.
    #[allow(dead_code)]
    pub fn send_json(&self, value: &Value) {
        self.ws.send_with_str(&value.to_string()).ok();
    }
}
