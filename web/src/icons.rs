//! Inline SVG icon set — lucide-style 24×24 strokes rendered directly in
//! rsx (no image assets, no JS). Path data is the standard lucide glyph set
//! (ISC license); stroke widths follow open-webui's usage per context.

use dioxus::prelude::*;

#[component]
pub fn Pencil(class: String) -> Element {
    rsx! {
        svg { class, view_box: "0 0 24 24", fill: "none", stroke: "currentColor",
            stroke_width: "1.9", stroke_linecap: "round", stroke_linejoin: "round",
            path { d: "M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z" }
            path { d: "m15 5 4 4" }
        }
    }
}

#[component]
pub fn Search(class: String) -> Element {
    rsx! {
        svg { class, view_box: "0 0 24 24", fill: "none", stroke: "currentColor",
            stroke_width: "1.9", stroke_linecap: "round", stroke_linejoin: "round",
            circle { cx: "11", cy: "11", r: "8" }
            path { d: "m21 21-4.3-4.3" }
        }
    }
}

#[component]
pub fn PanelLeft(class: String) -> Element {
    rsx! {
        svg { class, view_box: "0 0 24 24", fill: "none", stroke: "currentColor",
            stroke_width: "1.9", stroke_linecap: "round", stroke_linejoin: "round",
            rect { x: "3", y: "3", width: "18", height: "18", rx: "2" }
            path { d: "M9 3v18" }
        }
    }
}

#[component]
pub fn Plus(class: String) -> Element {
    rsx! {
        svg { class, view_box: "0 0 24 24", fill: "none", stroke: "currentColor",
            stroke_width: "2", stroke_linecap: "round", stroke_linejoin: "round",
            path { d: "M5 12h14" }
            path { d: "M12 5v14" }
        }
    }
}

#[component]
pub fn ChevronDown(class: String) -> Element {
    rsx! {
        svg { class, view_box: "0 0 24 24", fill: "none", stroke: "currentColor",
            stroke_width: "2.5", stroke_linecap: "round", stroke_linejoin: "round",
            path { d: "m6 9 6 6 6-6" }
        }
    }
}

#[component]
pub fn ChevronLeft(class: String) -> Element {
    rsx! {
        svg { class, view_box: "0 0 24 24", fill: "none", stroke: "currentColor",
            stroke_width: "2.5", stroke_linecap: "round", stroke_linejoin: "round",
            path { d: "m15 18-6-6 6-6" }
        }
    }
}

#[component]
pub fn ChevronRight(class: String) -> Element {
    rsx! {
        svg { class, view_box: "0 0 24 24", fill: "none", stroke: "currentColor",
            stroke_width: "2.5", stroke_linecap: "round", stroke_linejoin: "round",
            path { d: "m9 18 6-6-6-6" }
        }
    }
}

#[component]
pub fn ArrowUp(class: String) -> Element {
    rsx! {
        svg { class, view_box: "0 0 24 24", fill: "none", stroke: "currentColor",
            stroke_width: "2.5", stroke_linecap: "round", stroke_linejoin: "round",
            path { d: "m5 12 7-7 7 7" }
            path { d: "M12 19V5" }
        }
    }
}

#[component]
pub fn Copy(class: String) -> Element {
    rsx! {
        svg { class, view_box: "0 0 24 24", fill: "none", stroke: "currentColor",
            stroke_width: "2", stroke_linecap: "round", stroke_linejoin: "round",
            rect { x: "8", y: "8", width: "14", height: "14", rx: "2" }
            path { d: "M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2" }
        }
    }
}

#[component]
pub fn Trash(class: String) -> Element {
    rsx! {
        svg { class, view_box: "0 0 24 24", fill: "none", stroke: "currentColor",
            stroke_width: "2", stroke_linecap: "round", stroke_linejoin: "round",
            path { d: "M3 6h18" }
            path { d: "M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6" }
            path { d: "M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" }
        }
    }
}

#[component]
pub fn X(class: String) -> Element {
    rsx! {
        svg { class, view_box: "0 0 24 24", fill: "none", stroke: "currentColor",
            stroke_width: "2", stroke_linecap: "round", stroke_linejoin: "round",
            path { d: "M18 6 6 18" }
            path { d: "m6 6 12 12" }
        }
    }
}

#[component]
pub fn LogOut(class: String) -> Element {
    rsx! {
        svg { class, view_box: "0 0 24 24", fill: "none", stroke: "currentColor",
            stroke_width: "2", stroke_linecap: "round", stroke_linejoin: "round",
            path { d: "M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4" }
            path { d: "m16 17 5-5-5-5" }
            path { d: "M21 12H9" }
        }
    }
}

/// Filled ellipsis (chat item / navbar "more" button).
#[component]
pub fn Ellipsis(class: String) -> Element {
    rsx! {
        svg { class, view_box: "0 0 24 24", fill: "currentColor", stroke: "none",
            circle { cx: "5", cy: "12", r: "1.6" }
            circle { cx: "12", cy: "12", r: "1.6" }
            circle { cx: "19", cy: "12", r: "1.6" }
        }
    }
}

/// Filled stop square (generation stop state — reserved; send shows it disabled).
#[component]
pub fn Stop(class: String) -> Element {
    rsx! {
        svg { class, view_box: "0 0 24 24", fill: "currentColor", stroke: "none",
            rect { x: "6.5", y: "6.5", width: "11", height: "11", rx: "2" }
        }
    }
}

/// "Suggested" bolt (open-webui Suggestions header).
#[component]
pub fn Zap(class: String) -> Element {
    rsx! {
        svg { class, view_box: "0 0 24 24", fill: "none", stroke: "currentColor",
            stroke_width: "1.9", stroke_linecap: "round", stroke_linejoin: "round",
            path { d: "M4 14a1 1 0 0 1-.78-1.63l9.9-10.2a.5.5 0 0 1 .86.46l-1.92 6.02A1 1 0 0 0 13 10h7a1 1 0 0 1 .78 1.63l-9.9 10.2a.5.5 0 0 1-.86-.46l1.92-6.02A1 1 0 0 0 11 14z" }
        }
    }
}
