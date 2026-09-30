//! Mermaid diagram rendering (DECISIONS D-005): ```mermaid fences are
//! rendered to SVG by sebastian (pure Rust, pixel-compatible with
//! mermaid.js). Runs on wasm and on the host (unit tests).
//!
//! Failure degrades to the original code block plus an error note — a
//! broken diagram must never erase the user's source.
//!
//! 覆盖矩阵（tests below）:
//! ✅ sebastian 核心：flowchart / pie → SVG；非法源码 → Err（host 可测）
//! ✅ DOM 替换逻辑（wasm-only）按 data-mermaid 标记幂等；成功替换 <pre>，
//!    失败保留源码 + 错误提示（浏览器冒烟在 T5）
//! ⛔ 刻意不覆盖：18 种图型逐一渲染（sebastian 上游回归测试体系负责）

use wasm_bindgen::JsCast;

/// Renders every not-yet-processed `pre > code.language-mermaid` block in
/// the document. Idempotent via a `data-mermaid` marker attribute.
pub async fn render_mermaid_blocks() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };
    let Ok(nodes) = document.query_selector_all("pre > code.language-mermaid") else {
        return;
    };
    for index in 0..nodes.length() {
        let Some(node) = nodes.get(index) else {
            continue;
        };
        let Ok(code) = node.dyn_into::<web_sys::Element>() else {
            continue;
        };
        if code.get_attribute("data-mermaid").is_some() {
            continue;
        }
        let _ = code.set_attribute("data-mermaid", "1");
        let source = code.text_content().unwrap_or_default();
        // unique svg element ids per render (mermaid ids nodes/styles)
        let id = format!("mermaid-{}", crate::api::uuid_v4());
        let Some(pre) = code.parent_element() else {
            continue;
        };
        match sebastian::render_diagram(&source, &id) {
            Ok(svg) => {
                if let Ok(wrapper) = document.create_element("div") {
                    wrapper.set_class_name("mermaid-svg");
                    wrapper.set_inner_html(&svg);
                    let _ = pre.replace_with_with_node_1(&wrapper);
                }
            }
            Err(error) => {
                if let Ok(note) = document.create_element("div") {
                    note.set_class_name("mermaid-error");
                    note.set_text_content(Some(&format!("mermaid render failed: {error}")));
                    let _ = pre.append_child(&note);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    

    #[test]
    fn flowchart_renders_svg() {
        let svg = sebastian::render_diagram(
            "flowchart TD\n    A[Start] --> B{Is it?}\n    B -->|Yes| C[OK]\n    B -->|No| D[End]",
            "test-flowchart",
        )
        .expect("flowchart renders");
        assert!(svg.contains("<svg"), "{svg}");
    }

    #[test]
    fn pie_renders_svg() {
        let svg = sebastian::render_diagram(
            "pie title Pets\n    \"Dogs\" : 386\n    \"Cats\" : 85",
            "test-pie",
        )
        .expect("pie renders");
        assert!(svg.contains("<svg"), "{svg}");
    }

    #[test]
    fn invalid_source_is_an_error() {
        assert!(sebastian::render_diagram("not a diagram at all ???", "test-bad").is_err());
    }
}
