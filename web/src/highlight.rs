//! Syntax-highlight backfill for finished code blocks (T3): converts
//! `/api/v1/utils/highlight` span triplets into HTML, and applies them to
//! `pre > code[class*="language-"]` elements after markdown render.
//!
//! DOM access lives in `backfill_code_blocks` (wasm-only); the span→HTML
//! conversion is pure and host-testable.
//!
//! 覆盖矩阵（tests below）:
//! ✅ span 包裹与类名映射（dotted 捕获名取首段 → hl-*）
//! ✅ 重叠 span 丢弃（先到先得，外层优先）、越界/逆序 span 丢弃
//! ✅ 文本 HTML 转义（<>&、属性注入）
//! ✅ 非 ASCII 字节偏移（char boundary 校验后切片）
//! ⛔ 刻意不覆盖：DOM 副作用（浏览器冒烟在 T5）、主题色值（CSS 职责）

use dioxus::prelude::spawn;
use wasm_bindgen::JsCast;
use serde_json::json;

use crate::api;

/// Maps a dotted capture name to a CSS class: first segment wins
/// (`function.method.call` → `hl-function`). Unknown segments produce an
/// inert class (no CSS rule), never invalid HTML.
pub fn class_for(name: &str) -> String {
    format!("hl-{}", name.split('.').next().unwrap_or("other"))
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Builds highlighted inner HTML for `code` from span triplets
/// `(start, end, capture_name)` in byte offsets. Overlapping spans are
/// skipped (earlier span wins), all text is escaped.
pub fn spans_to_html(code: &str, spans: &[(usize, usize, String)]) -> String {
    let mut out = String::with_capacity(code.len() + spans.len() * 24);
    let mut cursor = 0usize;
    for (start, end, name) in spans {
        if *start < cursor || *end > code.len() || *start >= *end {
            continue;
        }
        if !code.is_char_boundary(*start) || !code.is_char_boundary(*end) {
            continue;
        }
        out.push_str(&escape_html(&code[cursor..*start]));
        out.push_str(&format!(
            "<span class=\"{}\">{}</span>",
            class_for(name),
            escape_html(&code[*start..*end])
        ));
        cursor = *end;
    }
    out.push_str(&escape_html(&code[cursor..]));
    out
}

/// Scans the document for rendered code blocks and backfills spans from the
/// highlight endpoint. Idempotent via a `data-hl` marker attribute; blocks
/// in unsupported languages keep their plain rendering.
pub async fn backfill_code_blocks() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };
    let Ok(nodes) = document.query_selector_all("pre > code[class*='language-']") else {
        return;
    };
    for index in 0..nodes.length() {
        let Some(node) = nodes.get(index) else {
            continue;
        };
        let Ok(element) = node.dyn_into::<web_sys::Element>() else {
            continue;
        };
        if element.get_attribute("data-hl").as_deref() == Some("1") {
            continue;
        }
        let _ = element.set_attribute("data-hl", "1");
        let Some(language) = element
            .class_name()
            .split_whitespace()
            .find_map(|c| c.strip_prefix("language-").map(str::to_string))
        else {
            continue;
        };
        let code = element.text_content().unwrap_or_default();
        let element = element.clone();
        spawn(async move {
            let Ok((200, response)) = api::api_post(
                "/api/v1/utils/highlight",
                &json!({"code": code, "language": language}),
            )
            .await
            else {
                return;
            };
            if response["unsupported"] == json!(true) {
                return;
            }
            let Some(spans) = response["spans"].as_array().map(|items| {
                items
                    .iter()
                    .filter_map(|t| {
                        let triplet = t.as_array()?;
                        Some((
                            triplet.first()?.as_u64()? as usize,
                            triplet.get(1)?.as_u64()? as usize,
                            triplet.get(2)?.as_str()?.to_string(),
                        ))
                    })
                    .collect::<Vec<(usize, usize, String)>>()
            }) else {
                return;
            };
            let html = spans_to_html(&code, &spans);
            element.set_inner_html(&html);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_name_first_segment() {
        assert_eq!(class_for("keyword"), "hl-keyword");
        assert_eq!(class_for("function.method.call"), "hl-function");
        assert_eq!(class_for("string.special"), "hl-string");
        assert_eq!(class_for(""), "hl-");
    }

    #[test]
    fn spans_wrap_and_escape() {
        let code = "let x = \"<a>\";";
        let spans = vec![(0, 3, "keyword".into()), (8, 13, "string".into())];
        let html = spans_to_html(code, &spans);
        assert_eq!(
            html,
            "<span class=\"hl-keyword\">let</span> x = <span class=\"hl-string\">&quot;&lt;a&gt;&quot;</span>;"
        );
    }

    #[test]
    fn overlapping_and_invalid_spans_are_skipped() {
        let code = "abc def";
        let spans = vec![
            (0, 3, "keyword".into()),
            (2, 5, "variable".into()),    // overlaps → skipped
            (4, 7, "keyword".into()),     // 4 < cursor 3? no, cursor=3 → kept
            (5, 2, "keyword".into()),     // reversed → skipped
            (100, 200, "keyword".into()), // out of bounds → skipped
        ];
        let html = spans_to_html(code, &spans);
        assert_eq!(
            html,
            "<span class=\"hl-keyword\">abc</span> <span class=\"hl-keyword\">def</span>"
        );
    }

    #[test]
    fn unicode_offsets_slice_by_byte() {
        // 中文 = 3 bytes each; keyword starts after the comment
        let code = "// 注释\nlet x;";
        let kw_start = code.find("let").unwrap();
        let spans = vec![(kw_start, kw_start + 3, "keyword".into())];
        let html = spans_to_html(code, &spans);
        assert!(html.contains("<span class=\"hl-keyword\">let</span>"));
        assert!(html.starts_with("// 注释\n"));
    }

    #[test]
    fn empty_input_yields_empty_escape() {
        assert_eq!(spans_to_html("", &[]), "");
        assert_eq!(spans_to_html("plain", &[]), "plain");
    }
}
