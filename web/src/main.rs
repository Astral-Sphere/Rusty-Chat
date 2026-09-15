use dioxus::prelude::*;

fn main() {
    launch(App);
}

#[component]
fn App() -> Element {
    // M1 replaces this with the router (/auth, /, /c/[id], /workspace, /admin…).
    rsx! {
        div { class: "min-h-screen flex items-center justify-center",
            h1 { class: "text-2xl font-semibold", "Rusty-Chat" }
            p { class: "text-gray-500", "frontend skeleton — M0" }
        }
    }
}
