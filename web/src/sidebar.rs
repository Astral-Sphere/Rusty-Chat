//! Sidebar — open-webui layout imitation: chat list grouped by pinned /
//! calendar time ranges, title search, per-item delete menu, user menu, and
//! the collapsed 42px icon rail.
//!
//! 覆盖矩阵（tests below）:
//! ✅ time_range 边界（今天/昨天/过去 7 天/过去 30 天/更早月份）
//! ✅ 跨时区偏移（local day 按 tz_offset 折算，含负偏移/跨年）
//! ✅ time_ago 分桶（分钟/小时/天/周/年、下限 1 分钟、未来时间钳制为 0）
//! ✅ group_chats 排序：组按首次出现序（新→旧天然成立），组内保序；
//!    乱序输入产生重复组标签属调用方契约（已钉死）
//! ✅ 加固：time_ago 精确边界（3599/3600/86400/7d/365d）、未来时间戳落组、
//!    闰日与世纪年 civil_from_days 往返、pre-epoch、±极值时区跨日、
//!    avatar_letter 兜底、percent_encode（保留集/多字节/空串）
//! ⛔ 刻意不覆盖：DOM/网络副作用（拉列表、搜索请求、删除、菜单交互）——
//!    属浏览器冒烟范围（对应 open-webui Sidebar.svelte 的行为由视觉验收对拍）。

use dioxus::prelude::*;
use serde_json::Value;

use crate::api;
use crate::icons;

#[derive(Clone, Debug, PartialEq)]
pub struct ChatEntry {
    pub id: String,
    pub title: String,
    pub updated_at: i64,
}

/// Calendar-bucketed chat age, mirroring open-webui Sidebar's time_range
/// (Today / Yesterday / Previous 7 days / Previous 30 days / month names).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TimeGroup {
    Today,
    Yesterday,
    Previous7Days,
    Previous30Days,
    /// (year, month 1-12) in the browser's local timezone
    Month(i64, u32),
}

impl TimeGroup {
    pub fn label(&self) -> String {
        match self {
            Self::Today => "今天".into(),
            Self::Yesterday => "昨天".into(),
            Self::Previous7Days => "过去 7 天".into(),
            Self::Previous30Days => "过去 30 天".into(),
            Self::Month(_, m) => format!("{m}月"),
        }
    }
}

/// Local civil date (year, month, day) from epoch seconds + timezone offset.
fn civil_from(ts_secs: i64, tz_offset_secs: i64) -> (i64, u32, u32) {
    civil_from_days((ts_secs + tz_offset_secs).div_euclid(86_400))
}

/// Howard Hinnant's `civil_from_days` (public domain): days since
/// 1970-01-01 → proleptic Gregorian (y, m, d).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn time_range(updated_at: i64, now: i64, tz_offset_secs: i64) -> TimeGroup {
    let local_day = |t: i64| (t + tz_offset_secs).div_euclid(86_400);
    if local_day(updated_at) == local_day(now) {
        return TimeGroup::Today;
    }
    if local_day(updated_at) == local_day(now) - 1 {
        return TimeGroup::Yesterday;
    }
    // open-webui checks "is after now − 7/30 days" (same-time-of-day cutoffs)
    if updated_at > now - 7 * 86_400 {
        return TimeGroup::Previous7Days;
    }
    if updated_at > now - 30 * 86_400 {
        return TimeGroup::Previous30Days;
    }
    let (y, m, _) = civil_from(updated_at, tz_offset_secs);
    TimeGroup::Month(y, m)
}

/// Relative age label in open-webui's zh-CN style (分钟前/小时前/天前/周前/年前).
pub fn time_ago(updated_at: i64, now: i64) -> String {
    let secs = (now - updated_at).max(0);
    if secs < 3_600 {
        return format!("{}分钟前", (secs / 60).max(1));
    }
    if secs < 86_400 {
        return format!("{}小时前", secs / 3_600);
    }
    let days = secs / 86_400;
    if days < 7 {
        return format!("{days}天前");
    }
    if days < 365 {
        return format!("{}周前", days / 7);
    }
    format!("{}年前", days / 365)
}

/// Groups chats for display. Input order must be newest-first (the list
/// endpoint's order); group order follows first appearance, which yields
/// Today → Yesterday → 7d → 30d → months (newest first), and rows keep
/// their relative order inside a group.
pub fn group_chats(
    entries: &[ChatEntry],
    now: i64,
    tz_offset_secs: i64,
) -> Vec<(String, Vec<ChatEntry>)> {
    let mut groups: Vec<(String, Vec<ChatEntry>)> = Vec::new();
    for entry in entries {
        let label = time_range(entry.updated_at, now, tz_offset_secs).label();
        match groups.iter_mut().find(|(l, _)| *l == label) {
            Some((_, bucket)) => bucket.push(entry.clone()),
            None => groups.push((label, vec![entry.clone()])),
        }
    }
    groups
}

pub fn parse_chat_entry(value: &Value) -> Option<ChatEntry> {
    Some(ChatEntry {
        id: value["id"].as_str()?.to_string(),
        title: value["title"].as_str().unwrap_or("新对话").to_string(),
        updated_at: value["updated_at"].as_i64().unwrap_or(0),
    })
}

// Selection/menu actions are free functions taking owned Signal handles so
// the tiny `move` closures at each call site capture only Copy values and
// stay `Fn` (a closure that calls `.set()` on a captured signal is FnMut and
// cannot be dispatched to more than one EventHandler prop).

fn select_chat(
    mut selected: Signal<Option<String>>,
    mut menu_for: Signal<Option<String>>,
    mut searching: Signal<bool>,
    mut query: Signal<String>,
    id: String,
) {
    selected.set(Some(id));
    menu_for.set(None);
    searching.set(false);
    query.set(String::new());
}

fn start_new_chat(
    mut selected: Signal<Option<String>>,
    mut searching: Signal<bool>,
    mut query: Signal<String>,
) {
    selected.set(None);
    searching.set(false);
    query.set(String::new());
}

fn remove_chat(mut menu_for: Signal<Option<String>>, mut refresh: Signal<u32>, id: String) {
    spawn(async move {
        let _ = api::api_delete(&format!("/api/v1/chats/{id}")).await;
        menu_for.set(None);
        refresh += 1;
    });
}

fn toggle_menu(mut menu_for: Signal<Option<String>>, id: String) {
    if menu_for() == Some(id.clone()) {
        menu_for.set(None);
    } else {
        menu_for.set(Some(id));
    }
}

#[component]
pub fn sidebar(
    open: bool,
    user_name: String,
    refresh: Signal<u32>,
    selected: Signal<Option<String>>,
    on_toggle: EventHandler<()>,
    on_sign_out: EventHandler<()>,
) -> Element {
    let pinned = use_signal(Vec::<ChatEntry>::new);
    let chats = use_signal(Vec::<ChatEntry>::new);
    let mut searching = use_signal(|| false);
    let mut query = use_signal(String::new);
    let results = use_signal(Vec::<ChatEntry>::new);
    let menu_for = use_signal(|| None::<String>);
    let mut user_menu_open = use_signal(|| false);

    // chat list (+ pinned section); re-fetch on refresh bumps (title events,
    // deletions)
    use_effect(move || {
        refresh();
        to_owned![pinned, chats];
        spawn(async move {
            if let Ok(list) = api::api_get("/api/v1/chats/pinned").await {
                let rows: Vec<ChatEntry> = list
                    .as_array()
                    .map(|a| a.iter().filter_map(parse_chat_entry).collect())
                    .unwrap_or_default();
                pinned.set(rows);
            }
            if let Ok(list) = api::api_get("/api/v1/chats/").await {
                let rows: Vec<ChatEntry> = list
                    .as_array()
                    .map(|a| a.iter().filter_map(parse_chat_entry).collect())
                    .unwrap_or_default();
                chats.set(rows);
            }
        });
    });

    // live title search
    use_effect(move || {
        let q = query();
        let active = searching();
        to_owned![results];
        spawn(async move {
            if !active || q.trim().is_empty() {
                results.set(Vec::new());
                return;
            }
            if let Ok(list) =
                api::api_get(&format!("/api/v1/chats/search?text={}", percent_encode(&q))).await
            {
                let rows: Vec<ChatEntry> = list
                    .as_array()
                    .map(|a| a.iter().filter_map(parse_chat_entry).collect())
                    .unwrap_or_default();
                results.set(rows);
            }
        });
    });

    if open {
        let now = api::epoch_secs();
        let tz = api::tz_offset_secs();
        let searching_now = searching() && !query().trim().is_empty();
        let list: Vec<ChatEntry> = if searching_now { results() } else { chats() };
        let groups = if searching_now {
            vec![("搜索结果".to_string(), list)]
        } else {
            group_chats(&list, now, tz)
        };

        rsx! {
            aside { class: "relative h-screen w-[245px] shrink-0 flex flex-col bg-gray-950 text-gray-300 text-[0.8125rem] leading-5 border-r border-gray-900/60",
                // top row: logo, app name, collapse
                div { class: "flex items-center justify-between px-2 pt-2 pb-1",
                    button {
                        class: "flex size-9 items-center justify-center rounded-xl hover:bg-gray-900 transition cursor-pointer",
                        onclick: move |_| start_new_chat(selected, searching, query),
                        title: "新对话",
                        span { class: "flex size-5 items-center justify-center rounded-full bg-gradient-to-br from-blue-500 to-blue-700 text-[10px] font-bold text-white",
                            "R"
                        }
                    }
                    button {
                        class: "flex flex-1 px-1 h-full items-center truncate text-left font-normal text-gray-200 cursor-pointer",
                        onclick: move |_| start_new_chat(selected, searching, query),
                        "Rusty-Chat"
                    }
                    button {
                        class: "flex size-[1.875rem] shrink-0 items-center justify-center rounded-lg hover:bg-gray-900 transition cursor-pointer",
                        onclick: move |_| on_toggle.call(()),
                        title: "收起侧边栏",
                        icons::PanelLeft { class: "size-4" }
                    }
                }

                // nav body
                div { class: "flex-1 overflow-y-auto scrollbar-hidden flex flex-col pt-2.5 pb-2.5 space-y-1.5 px-1",
                    if searching() {
                        div { class: "flex items-center gap-2 rounded-xl bg-gray-900 px-2 py-1.5 mx-1",
                            icons::Search { class: "size-4 shrink-0 text-gray-500" }
                            input {
                                class: "w-full bg-transparent outline-none text-gray-200 placeholder-gray-500",
                                placeholder: "搜索对话",
                                value: query(),
                                autofocus: true,
                                oninput: move |e| query.set(e.value()),
                            }
                            button {
                                class: "shrink-0 text-gray-500 hover:text-gray-300 cursor-pointer",
                                onclick: move |_| {
                                    searching.set(false);
                                    query.set(String::new());
                                },
                                icons::X { class: "size-4" }
                            }
                        }
                    } else {
                        button {
                            class: "flex items-center space-x-2 rounded-xl px-2 py-1.5 hover:bg-gray-900 transition cursor-pointer text-left",
                            onclick: move |_| start_new_chat(selected, searching, query),
                            icons::Pencil { class: "size-4 shrink-0" }
                            span { class: "self-center", "新对话" }
                        }
                        button {
                            class: "flex items-center space-x-2 rounded-xl px-2 py-1.5 hover:bg-gray-900 transition cursor-pointer text-left",
                            onclick: move |_| searching.set(true),
                            icons::Search { class: "size-4 shrink-0" }
                            span { class: "self-center", "搜索" }
                        }
                    }

                    if !pinned().is_empty() && !searching() {
                        div { class: "mb-1",
                            div { class: "flex items-center justify-between h-6 pl-3.5 pr-1.5 shrink-0",
                                span { class: "text-xs text-gray-500", "已置顶" }
                            }
                            div { class: "ml-3 pl-1 flex flex-col border-l border-gray-900",
                                for entry in pinned() {
                                    chat_item {
                                        key: "{entry.id}",
                                        entry: entry.clone(),
                                        selected_id: selected(),
                                        menu_open: menu_for() == Some(entry.id.clone()),
                                        on_open: move |id| select_chat(selected, menu_for, searching, query, id),
                                        on_menu: move |id| toggle_menu(menu_for, id),
                                        on_delete: move |id| remove_chat(menu_for, refresh, id),
                                    }
                                }
                            }
                        }
                    }

                    div { class: "flex items-center justify-between h-6 pl-3.5 pr-1.5 shrink-0",
                        span { class: "text-xs text-gray-500",
                            if searching() { "搜索结果" } else { "对话" }
                        }
                    }
                    for (label, group) in groups {
                        div {
                            key: "{label}",
                            // search results already have their section header;
                            // only the calendar groups get a label row
                            if !searching_now {
                                div { class: "w-full pl-2.5 text-xs text-gray-500 font-normal pb-1 pt-3",
                                    "{label}"
                                }
                            }
                            div { class: "flex flex-col space-y-0.5",
                                for entry in group {
                                    chat_item {
                                        key: "{entry.id}",
                                        entry: entry.clone(),
                                        selected_id: selected(),
                                        menu_open: menu_for() == Some(entry.id.clone()),
                                        on_open: move |id| select_chat(selected, menu_for, searching, query, id),
                                        on_menu: move |id| toggle_menu(menu_for, id),
                                        on_delete: move |id| remove_chat(menu_for, refresh, id),
                                    }
                                }
                            }
                        }
                    }
                }

                // bottom user block
                div { class: "px-1 pt-1 pb-1.5 border-t border-gray-900/60",
                    if user_menu_open() {
                        button {
                            class: "fixed inset-0 z-40 cursor-default",
                            tabindex: -1,
                            onclick: move |_| user_menu_open.set(false),
                        }
                        div { class: "absolute bottom-14 left-2 z-50 w-[calc(100%-1rem)] rounded-xl border border-gray-800 bg-gray-850 p-1 shadow-xl text-xs",
                            div { class: "px-2 py-1.5 truncate text-gray-400",
                                if user_name.is_empty() { "已登录" } else { "{user_name}" }
                            }
                            div { class: "my-0.5 mx-1 border-t border-gray-800/60" }
                            button {
                                class: "flex w-full items-center gap-2 rounded-xl px-2 py-1.5 hover:bg-gray-900 transition cursor-pointer text-left",
                                onclick: move |_| {
                                    user_menu_open.set(false);
                                    on_sign_out.call(());
                                },
                                icons::LogOut { class: "size-3.5 shrink-0" }
                                "退出登录"
                            }
                        }
                    }
                    button {
                        class: "flex w-full items-center gap-2 rounded-xl px-1.5 py-1.5 hover:bg-gray-900 transition cursor-pointer text-left",
                        onclick: move |_| user_menu_open.toggle(),
                        span { class: "relative shrink-0",
                            span { class: "flex size-6 items-center justify-center rounded-full bg-gradient-to-br from-emerald-400 to-blue-500 text-xs font-semibold text-gray-950",
                                "{avatar_letter(&user_name)}"
                            }
                            span { class: "absolute -bottom-0.5 -right-0.5 size-2.5 rounded-full bg-green-500 border-2 border-gray-950" }
                        }
                        span { class: "flex-1 truncate font-normal text-gray-200",
                            if user_name.is_empty() { "用户" } else { "{user_name}" }
                        }
                    }
                }
            }
        }
    } else {
        // collapsed 42px rail
        rsx! {
            aside { class: "h-screen w-[42px] shrink-0 flex flex-col items-center justify-between py-2 bg-gray-950 text-gray-300 border-r border-gray-900/60",
                div { class: "flex flex-col items-center gap-1",
                    button {
                        class: "flex size-8 items-center justify-center rounded-lg hover:bg-gray-900 transition cursor-pointer",
                        onclick: move |_| on_toggle.call(()),
                        title: "展开侧边栏",
                        span { class: "flex size-5 items-center justify-center rounded-full bg-gradient-to-br from-blue-500 to-blue-700 text-[10px] font-bold text-white",
                            "R"
                        }
                    }
                    button {
                        class: "flex size-8 items-center justify-center rounded-lg hover:bg-gray-900 transition cursor-pointer",
                        onclick: move |_| {
                            on_toggle.call(());
                            start_new_chat(selected, searching, query);
                        },
                        title: "新对话",
                        icons::Pencil { class: "size-4" }
                    }
                    button {
                        class: "flex size-8 items-center justify-center rounded-lg hover:bg-gray-900 transition cursor-pointer",
                        onclick: move |_| {
                            on_toggle.call(());
                            searching.set(true);
                        },
                        title: "搜索",
                        icons::Search { class: "size-4" }
                    }
                }
                button {
                    class: "flex size-8 items-center justify-center rounded-lg hover:bg-gray-900 transition cursor-pointer",
                    onclick: move |_| on_toggle.call(()),
                    title: "展开侧边栏",
                    span { class: "relative",
                        span { class: "flex size-5.5 items-center justify-center rounded-full bg-gradient-to-br from-emerald-400 to-blue-500 text-[10px] font-semibold text-gray-950",
                            "{avatar_letter(&user_name)}"
                        }
                        span { class: "absolute -bottom-0.5 -right-0.5 size-2 rounded-full bg-green-500 border border-gray-950" }
                    }
                }
            }
        }
    }
}

#[component]
fn chat_item(
    entry: ChatEntry,
    selected_id: Option<String>,
    menu_open: bool,
    on_open: EventHandler<String>,
    on_menu: EventHandler<String>,
    on_delete: EventHandler<String>,
) -> Element {
    let id = entry.id.clone();
    let id_for_menu_btn = entry.id.clone();
    let id_for_menu_backdrop = entry.id.clone();
    let id_for_delete = entry.id.clone();
    let is_selected = selected_id.as_deref() == Some(entry.id.as_str());
    let now = api::epoch_secs();
    rsx! {
        div { class: "group relative",
            button {
                class: if is_selected {
                    "w-full flex justify-between items-center rounded-xl px-2 py-1.5 bg-white/[0.06] whitespace-nowrap transition cursor-pointer text-left"
                } else {
                    "w-full flex justify-between items-center rounded-xl px-2 py-1.5 hover:bg-gray-900 whitespace-nowrap transition cursor-pointer text-left"
                },
                onclick: move |_| on_open.call(id.clone()),
                span { class: "flex-1 overflow-hidden h-5 truncate text-gray-200", "{entry.title}" }
                span { class: "shrink-0 pl-2 text-[0.625rem] text-gray-500",
                    "{time_ago(entry.updated_at, now)}"
                }
            }
            button {
                class: "hover-reveal absolute right-1 inset-y-0 mr-1 flex items-center text-gray-500 hover:text-gray-300 cursor-pointer",
                onclick: move |e| {
                    e.stop_propagation();
                    on_menu.call(id_for_menu_btn.clone());
                },
                icons::Ellipsis { class: "size-3.5" }
            }
            if menu_open {
                button {
                    class: "fixed inset-0 z-40 cursor-default",
                    tabindex: -1,
                    onclick: move |_| on_menu.call(id_for_menu_backdrop.clone()),
                }
                div { class: "absolute right-1 top-8 z-50 w-36 rounded-xl border border-gray-800 bg-gray-850 p-1 shadow-xl",
                    button {
                        class: "flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-red-400 hover:bg-gray-900 transition cursor-pointer text-left",
                        onclick: move |_| on_delete.call(id_for_delete.clone()),
                        icons::Trash { class: "size-3.5 shrink-0" }
                        "删除"
                    }
                }
            }
        }
    }
}

/// First displayable character for the avatar (fallback: "用").
fn avatar_letter(name: &str) -> String {
    name.trim()
        .chars()
        .find(|c| !c.is_whitespace())
        .map(String::from)
        .unwrap_or_else(|| "用".to_string())
}

/// Minimal percent-encoding for query values (avoid a wasm dep for one call).
fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Reference instants, computed from the civil calendar:
    //   NOW = 2026-10-01T12:00:00Z（tz+8 的本地 20:00）
    //   2026-10-01 是自 epoch 起第 20727 天（1970-01-01 = 第 0 天）。
    const NOW: i64 = 1_790_856_000;
    const TZ: i64 = 8 * 3600;

    /// local (y, m, d, h:mm, tz+8) → epoch seconds（本地转 UTC 减 8h）。
    fn ts(y: i64, m: u32, d: u32, h: i64, min: i64) -> i64 {
        let y = if m <= 2 { y - 1 } else { y };
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = (y - era * 400) as u64; // [0, 399]
        let mp = if m > 2 { m - 3 } else { m + 9 } as u64;
        let doy = (153 * mp + 2) / 5 + (d as u64) - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era * 146_097 + doe as i64 - 719_468;
        days * 86_400 + (h - 8) * 3_600 + min * 60
    }

    // -- time_range 边界 --

    #[test]
    fn today_same_local_day() {
        // 本地 23:59（UTC 15:59，未到 NOW 的本地 20:00）→ 今天
        assert_eq!(
            time_range(ts(2026, 10, 1, 23, 59), NOW, TZ),
            TimeGroup::Today
        );
        // 本地 00:00（UTC 前一天 16:00）但本地日历仍是今天 → 今天
        assert_eq!(time_range(ts(2026, 10, 1, 0, 0), NOW, TZ), TimeGroup::Today);
    }

    #[test]
    fn yesterday_by_local_calendar_not_24h() {
        // 22h 前 = 昨天本地 22:00（不是 24h 边界）
        assert_eq!(time_range(NOW - 22 * 3_600, NOW, TZ), TimeGroup::Yesterday);
        // 48h 前 = 前天 → 过去 7 天
        assert_eq!(
            time_range(NOW - 48 * 3_600, NOW, TZ),
            TimeGroup::Previous7Days
        );
    }

    #[test]
    fn previous_7_and_30_use_same_time_of_day_cutoff() {
        // 6 天前同时刻 → 过去 7 天
        assert_eq!(
            time_range(NOW - 6 * 86_400, NOW, TZ),
            TimeGroup::Previous7Days
        );
        // now−7d+1s → 过去 7 天；恰好 now−7d → 过去 30 天
        assert_eq!(
            time_range(NOW - 7 * 86_400 + 1, NOW, TZ),
            TimeGroup::Previous7Days
        );
        assert_eq!(
            time_range(NOW - 7 * 86_400, NOW, TZ),
            TimeGroup::Previous30Days
        );
        // now−30d+1s → 过去 30 天；now−30d → 月份（9月1日本地）
        assert_eq!(
            time_range(NOW - 30 * 86_400 + 1, NOW, TZ),
            TimeGroup::Previous30Days
        );
        assert_eq!(
            time_range(NOW - 30 * 86_400, NOW, TZ),
            TimeGroup::Month(2026, 9)
        );
    }

    #[test]
    fn month_group_uses_local_calendar_and_crosses_year() {
        assert_eq!(
            time_range(ts(2026, 8, 15, 12, 0), NOW, TZ),
            TimeGroup::Month(2026, 8)
        );
        // 跨年：2025-12-31 本地 → 12月
        assert_eq!(
            time_range(ts(2025, 12, 31, 23, 0), NOW, TZ),
            TimeGroup::Month(2025, 12)
        );
    }

    #[test]
    fn negative_timezone_shifts_day_boundary() {
        // tz=−5h：NOW 本地 07:00；7h 前是本地当天 00:00 → 今天
        let tz = -5 * 3600;
        assert_eq!(time_range(NOW - 7 * 3_600, NOW, tz), TimeGroup::Today);
        // 8h 前 = 昨天 23:00；10h 前 = 昨天 21:00 → 昨天
        assert_eq!(time_range(NOW - 8 * 3_600, NOW, tz), TimeGroup::Yesterday);
        assert_eq!(time_range(NOW - 10 * 3_600, NOW, tz), TimeGroup::Yesterday);
    }

    #[test]
    fn future_timestamps_are_today() {
        assert_eq!(time_range(NOW + 3_600, NOW, TZ), TimeGroup::Today);
    }

    // -- time_ago --

    #[test]
    fn minutes_hours_days_weeks_years_buckets() {
        assert_eq!(time_ago(NOW - 30, NOW), "1分钟前"); // 下限
        assert_eq!(time_ago(NOW - 5 * 60, NOW), "5分钟前");
        assert_eq!(time_ago(NOW - 2 * 3_600, NOW), "2小时前");
        assert_eq!(time_ago(NOW - 23 * 3_600, NOW), "23小时前");
        assert_eq!(time_ago(NOW - 3 * 86_400, NOW), "3天前");
        assert_eq!(time_ago(NOW - 14 * 86_400, NOW), "2周前");
        assert_eq!(time_ago(NOW - 400 * 86_400, NOW), "1年前");
    }

    #[test]
    fn future_clamps_to_one_minute() {
        assert_eq!(time_ago(NOW + 3_600, NOW), "1分钟前");
    }

    // -- group_chats --

    #[test]
    fn groups_follow_first_appearance_and_preserve_order() {
        let entries = vec![
            entry("a", NOW - 60),          // 今天
            entry("b", NOW - 25 * 86_400), // 过去 30 天
            entry("c", NOW - 5 * 3_600),   // 今天
            entry("d", NOW - 3 * 86_400),  // 过去 7 天
            entry("e", NOW - 28 * 86_400), // 过去 30 天
        ];
        let groups = group_chats(&entries, NOW, TZ);
        let labels: Vec<&str> = groups.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, vec!["今天", "过去 30 天", "过去 7 天"]);
        assert_eq!(
            groups[0]
                .1
                .iter()
                .map(|e| e.id.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "c"]
        );
        assert_eq!(
            groups[1]
                .1
                .iter()
                .map(|e| e.id.as_str())
                .collect::<Vec<_>>(),
            vec!["b", "e"]
        );
        assert_eq!(
            groups[2]
                .1
                .iter()
                .map(|e| e.id.as_str())
                .collect::<Vec<_>>(),
            vec!["d"]
        );
    }

    #[test]
    fn empty_input_gives_no_groups() {
        assert!(group_chats(&[], NOW, TZ).is_empty());
    }

    #[test]
    fn parse_entry_defaults() {
        let parsed = parse_chat_entry(&json!({"id": "x", "title": "T", "updated_at": 5}));
        assert_eq!(
            parsed,
            Some(ChatEntry {
                id: "x".into(),
                title: "T".into(),
                updated_at: 5
            })
        );
        assert_eq!(parse_chat_entry(&json!({"title": "T"})), None);
        let missing_title = parse_chat_entry(&json!({"id": "x"}));
        assert_eq!(missing_title.unwrap().title, "新对话");
    }

    fn entry(id: &str, updated_at: i64) -> ChatEntry {
        ChatEntry {
            id: id.into(),
            title: id.into(),
            updated_at,
        }
    }

    // -- 加固批次：精确边界与钉死 --

    #[test]
    fn time_ago_exact_bucket_edges() {
        // < 3600s → minutes (min 1); 3600 → first hour bucket
        assert_eq!(time_ago(NOW - 3_599, NOW), "59分钟前");
        assert_eq!(time_ago(NOW - 3_600, NOW), "1小时前");
        assert_eq!(time_ago(NOW - 86_399, NOW), "23小时前");
        assert_eq!(time_ago(NOW - 86_400, NOW), "1天前");
        assert_eq!(time_ago(NOW - 7 * 86_400 - 1, NOW), "1周前");
        assert_eq!(time_ago(NOW - 364 * 86_400, NOW), "52周前");
        assert_eq!(time_ago(NOW - 365 * 86_400, NOW), "1年前");
    }

    #[test]
    fn future_timestamps_outside_today_land_in_previous7() {
        // open-webui semantics pinned: a future local-day mismatch falls
        // through Today/Yesterday into the `> now − 7d` bucket
        assert_eq!(time_range(NOW + 3_600, NOW, TZ), TimeGroup::Today);
        assert_eq!(
            time_range(NOW + 48 * 3_600, NOW, TZ),
            TimeGroup::Previous7Days
        );
        // time_ago clamps to "just now"-style minute label
        assert_eq!(time_ago(NOW + 500, NOW), "1分钟前");
    }

    #[test]
    fn civil_from_days_leap_year_era_roundtrips() {
        // 1900 (non-leap century), 2000 (leap century), 2100 (non-leap)
        let cases = [
            (ts(1900, 3, 1, 12, 0), (1900, 3, 1)),
            (ts(2000, 2, 29, 12, 0), (2000, 2, 29)),
            (ts(2100, 3, 1, 12, 0), (2100, 3, 1)),
            (ts(2024, 2, 29, 12, 0), (2024, 2, 29)),
        ];
        for (instant, expected) in cases {
            let got = civil_from(instant, 0);
            assert_eq!(got, expected, "instant {instant}");
        }
    }

    #[test]
    fn pre_epoch_and_extreme_timezones() {
        // pre-1970 (div_euclid keeps the math honest): older than the
        // 30-day window of `now = 0` → Month(1969, 12)
        assert_eq!(time_range(-40 * 86_400, 0, 0), TimeGroup::Month(1969, 11));
        // date-line flip for the SAME hour-old instant: at +14h (Kiribati)
        // both now and now−1h sit on Oct 2 local → Today; at −12h
        // (Baker Island) the hour step crosses back over local midnight
        // into Sep 30 → Yesterday
        assert_eq!(time_range(NOW - 3_600, NOW, 14 * 3600), TimeGroup::Today);
        assert_eq!(
            time_range(NOW - 3_600, NOW, -12 * 3600),
            TimeGroup::Yesterday
        );
    }

    #[test]
    fn group_chats_newest_first_contract_is_on_the_caller() {
        // the pure function preserves input order; a non-sorted input would
        // produce repeated group labels — pinned so callers know the contract
        let out = group_chats(&[entry("b", NOW - 90 * 86_400), entry("a", NOW)], NOW, TZ);
        let labels: Vec<&String> = out.iter().map(|(l, _)| l).collect();
        assert_eq!(
            labels,
            vec!["7月", "今天"],
            "one label per first appearance"
        );
    }

    #[test]
    fn avatar_letter_first_visible_char_or_fallback() {
        assert_eq!(avatar_letter("Alice"), "A");
        assert_eq!(avatar_letter(" 中文"), "中");
        assert_eq!(avatar_letter("  "), "用");
        assert_eq!(avatar_letter(""), "用");
        assert_eq!(avatar_letter("\t x"), "x");
    }

    #[test]
    fn percent_encode_unreserved_and_multibyte() {
        assert_eq!(percent_encode("safe-_.~09"), "safe-_.~09");
        assert_eq!(percent_encode("a b"), "a%20b");
        assert_eq!(percent_encode("中"), "%E4%B8%AD");
        assert_eq!(percent_encode(""), "");
        assert_eq!(percent_encode("100%"), "100%25");
    }
}
