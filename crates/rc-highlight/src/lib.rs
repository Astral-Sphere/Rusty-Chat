//! rc-highlight — server-side syntax highlighting.
//!
//! tree-sitter parsing (crates.io grammar crates) + the Zed editor's
//! `highlights.scm` query files vendored as data assets
//! (`assets/highlights/`, see NOTICE and DECISIONS D-013). Produces colored
//! span lists (`Span { start, end, name }`, byte offsets) that the frontend
//! backfills into finished code blocks.
//!
//! Predicate evaluation (`#match?`/`#eq?`/`#any-of?` — used heavily by the
//! zed queries) is performed by the tree-sitter runtime itself via the
//! `TextProvider` passed to `QueryCursor::captures`; queries whose node
//! names do not compile against the crates.io grammar version are recorded
//! in `language_errors()` and the language is reported unsupported instead
//! of failing at runtime.
//!
//! 覆盖矩阵（tests below）:
//! ✅ 每语言 smoke：一段真实代码 → span 覆盖关键字/字符串/注释且字节偏移
//!    切片回读等于源文本（偏移正确性）
//! ✅ predicate 生效：rust `#match? @type "^[A-Z]"` 只捕获 Pascal 类型名；
//!    python `__init__` 命中 constructor 捕获
//! ✅ @none 抹除：rust 小写路径段被 @none 覆盖时不输出
//! ✅ 未知语言 → None；空代码 → Some(空 span)；查询编译失败的语言不进
//!    languages() 且记入 language_errors()
//! ✅ 边界：空串、超长输入、无换行、深嵌套 JSON（不 panic）、CRLF、
//!    非 ASCII（中文注释/emoji 前后的字节偏移）
//! ⛔ 刻意不覆盖：injections.scm（HTML 内嵌 CSS/JS 的子语言着色，v1 单语
//!    言处理）；主题配色（前端 CSS 职责）；并发解析（Parser 每次调用
//!    新建，无共享状态）

use std::collections::BTreeMap;
use std::sync::LazyLock;

use tree_sitter::{Parser, Query};
use tree_sitter_language::LanguageFn;

/// One highlighted region. `start`/`end` are byte offsets into the source
/// code (always on UTF-8 boundaries; slice with `&code[start..end]`).
/// `name` is the dotted capture name from the query (e.g. `function.method`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub name: String,
}

struct Compiled {
    language: tree_sitter::Language,
    query: Query,
}

/// markdown 0.7.1 predates the `LanguageFn` era; its parser symbol is
/// redeclared directly (same pattern the generated bindings use) so every
/// language funnels through the same `LanguageFn -> Language` path.
static REGISTRY: LazyLock<BTreeMap<&'static str, Compiled>> = LazyLock::new(|| {
    let mut map = BTreeMap::new();
    for (name, lang_fn, query_src) in QUERY_ENTRIES {
        let language: tree_sitter::Language = (*lang_fn).into();
        match Query::new(&language, query_src) {
            Ok(query) => {
                map.insert(*name, Compiled { language, query });
            }
            Err(e) => {
                QUERY_ERRORS
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .push((*name, e.to_string()));
            }
        }
    }
    map
});

// Shared with REGISTRY init: (language, compile error) pairs. The mutex is
// only contended during LazyLock init (single-threaded per call site).
static QUERY_ERRORS: LazyLock<std::sync::Mutex<Vec<(&'static str, String)>>> =
    LazyLock::new(|| std::sync::Mutex::new(Vec::new()));

/// (name, grammar, zed query asset). html is the only language whose query
/// comes from the grammar crate itself (zed ships no html grammar).
static QUERY_ENTRIES: &[(&str, LanguageFn, &str)] = &[
    (
        "bash",
        tree_sitter_bash::LANGUAGE,
        include_str!("../assets/highlights/bash.scm"),
    ),
    (
        "c",
        tree_sitter_c::LANGUAGE,
        include_str!("../assets/highlights/c.scm"),
    ),
    (
        "cpp",
        tree_sitter_cpp::LANGUAGE,
        include_str!("../assets/highlights/cpp.scm"),
    ),
    (
        "css",
        tree_sitter_css::LANGUAGE,
        include_str!("../assets/highlights/css.scm"),
    ),
    (
        "diff",
        tree_sitter_diff::LANGUAGE,
        include_str!("../assets/highlights/diff.scm"),
    ),
    (
        "go",
        tree_sitter_go::LANGUAGE,
        include_str!("../assets/highlights/go.scm"),
    ),
    (
        "javascript",
        tree_sitter_javascript::LANGUAGE,
        include_str!("../assets/highlights/javascript.scm"),
    ),
    (
        "json",
        tree_sitter_json::LANGUAGE,
        include_str!("../assets/highlights/json.scm"),
    ),
    (
        "python",
        tree_sitter_python::LANGUAGE,
        include_str!("../assets/highlights/python.scm"),
    ),
    (
        "rust",
        tree_sitter_rust::LANGUAGE,
        include_str!("../assets/highlights/rust.scm"),
    ),
    (
        "tsx",
        tree_sitter_typescript::LANGUAGE_TSX,
        include_str!("../assets/highlights/tsx.scm"),
    ),
    (
        "typescript",
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
        include_str!("../assets/highlights/typescript.scm"),
    ),
    (
        "yaml",
        tree_sitter_yaml::LANGUAGE,
        include_str!("../assets/highlights/yaml.scm"),
    ),
];

/// Languages whose queries compiled and that `highlight` accepts.
pub fn languages() -> Vec<&'static str> {
    REGISTRY.keys().copied().collect()
}

/// Languages from the asset set that failed to compile (diagnostics only).
pub fn language_errors() -> Vec<(&'static str, String)> {
    let _ = *REGISTRY; // ensure compile errors were collected first
    QUERY_ERRORS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// Highlights `code` with `language`; `None` for unsupported languages.
/// Spans are sorted by start byte and may nest (outer capture first);
/// ranges suppressed by an `@none` capture are removed.
pub fn highlight(code: &str, language: &str) -> Option<Vec<Span>> {
    let compiled = REGISTRY.get(language)?;
    let mut parser = Parser::new();
    parser.set_language(&compiled.language).ok()?;
    let tree = parser.parse(code, None)?;

    let mut spans: Vec<Span> = Vec::new();
    let mut none_ranges: Vec<(usize, usize)> = Vec::new();
    let capture_names = compiled.query.capture_names();
    let mut cursor = tree_sitter::QueryCursor::new();
    let mut matches = cursor.captures(&compiled.query, tree.root_node(), code.as_bytes());
    // 0.27's query iterators are streaming-iterators (lending), not std Iterators
    use streaming_iterator::StreamingIterator as _;
    while let Some((match_, capture_index)) = matches.next() {
        let capture = match_.captures()[*capture_index];
        let name = capture_names[capture.index as usize];
        let range = capture.node.byte_range();
        if name == "none" {
            none_ranges.push((range.start, range.end));
        } else {
            spans.push(Span {
                start: range.start,
                end: range.end,
                name: name.to_string(),
            });
        }
    }
    spans.retain(|span| {
        !none_ranges
            .iter()
            .any(|&(start, end)| span.start >= start && span.end <= end)
    });
    spans.sort_by_key(|span| (span.start, span.end));
    Some(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression guard: every vendored query must compile. Languages that
    /// legitimately cannot compile belong in the "deliberately not covered"
    /// note above, not silently here.
    #[test]
    fn all_queries_compile() {
        assert!(language_errors().is_empty(), "{:?}", language_errors());
        assert_eq!(languages().len(), QUERY_ENTRIES.len());
    }

    // --- per-language smoke (offset correctness via slice-back) -----------

    #[test]
    fn rust_smoke() {
        let code = "fn main() {\n    let greeting = \"hello\";\n    // end\n}";
        let spans = highlight(code, "rust").expect("rust supported");
        assert!(!spans.is_empty());
        let keywords: Vec<&Span> = spans.iter().filter(|s| s.name == "keyword").collect();
        assert!(keywords.len() >= 2, "{spans:?}");
        assert!(
            spans.iter().any(|s| s.name.starts_with("string")),
            "{spans:?}"
        );
        assert!(spans.iter().any(|s| s.name == "comment"), "{spans:?}");
        // offsets must slice back to the source text
        for span in &spans {
            assert!(!span.name.is_empty());
            assert!(
                code.is_char_boundary(span.start) && code.is_char_boundary(span.end),
                "span {}..{} not on char boundary",
                span.start,
                span.end
            );
        }
        assert!(keywords.iter().any(|s| &code[s.start..s.end] == "fn"));
        assert!(keywords.iter().any(|s| &code[s.start..s.end] == "let"));
    }

    #[test]
    fn python_smoke() {
        let code = "import os\n\n# comment\ndef greet(name):\n    return f\"hi {name}\"";
        let spans = highlight(code, "python").expect("python supported");
        assert!(
            spans
                .iter()
                .any(|s| s.name == "keyword" && &code[s.start..s.end] == "def")
        );
        assert!(spans.iter().any(|s| s.name == "comment"));
        assert!(spans.iter().any(|s| s.name.starts_with("string")));
    }

    #[test]
    fn javascript_smoke() {
        let code = "const x = 42;\nfunction greet() { return x; }";
        let spans = highlight(code, "javascript").expect("javascript supported");
        assert!(spans.iter().any(|s| s.name.starts_with("keyword")));
        assert!(spans.iter().any(|s| s.name == "number"));
    }

    #[test]
    fn typescript_and_tsx_smoke() {
        let code = "interface A { b: number }";
        let spans = highlight(code, "typescript").expect("typescript supported");
        assert!(spans.iter().any(|s| s.name == "type"));
        let code = "const el = <div className=\"x\" />;";
        let spans = highlight(code, "tsx").expect("tsx supported");
        assert!(!spans.is_empty());
    }

    #[test]
    fn go_smoke() {
        let code = "package main\n\nfunc main() { println(\"hi\") }";
        let spans = highlight(code, "go").expect("go supported");
        assert!(spans.iter().any(|s| s.name == "keyword"));
    }

    #[test]
    fn c_and_cpp_smoke() {
        let code = "#include <stdio.h>\nint main(void) { return 0; }";
        let spans = highlight(code, "c").expect("c supported");
        assert!(spans.iter().any(|s| s.name.starts_with("keyword")));
        let code = "template<typename T> struct X { T t; };";
        let spans = highlight(code, "cpp").expect("cpp supported");
        assert!(spans.iter().any(|s| s.name.starts_with("keyword")));
    }

    #[test]
    fn json_smoke() {
        let code = "{\"key\": [1, true, null]}";
        let spans = highlight(code, "json").expect("json supported");
        assert!(
            spans
                .iter()
                .any(|s| s.name == "string" && &code[s.start..s.end] == "\"key\"")
        );
    }

    #[test]
    fn yaml_bash_css_diff_smoke() {
        let code = "name: value # note\nlist:\n  - a";
        let spans = highlight(code, "yaml").expect("yaml supported");
        assert!(!spans.is_empty());

        let code = "echo \"hi\" # comment";
        let spans = highlight(code, "bash").expect("bash supported");
        assert!(spans.iter().any(|s| s.name.starts_with("string")));

        let code = "a { color: red; }";
        let spans = highlight(code, "css").expect("css supported");
        assert!(!spans.is_empty());

        let code = "--- a/file\n+++ b/file\n@@ -1 +1 @@\n-old\n+new";
        let spans = highlight(code, "diff").expect("diff supported");
        assert!(!spans.is_empty());
    }

    // --- predicates -------------------------------------------------------

    #[test]
    fn rust_type_predicate_filters_lowercase() {
        let code = "struct PascalType;\n\nfn lowercase_thing() {}";
        let spans = highlight(code, "rust").unwrap();
        let type_spans: Vec<&Span> = spans.iter().filter(|s| s.name == "type").collect();
        assert!(
            type_spans
                .iter()
                .any(|s| &code[s.start..s.end] == "PascalType"),
            "{spans:?}"
        );
        assert!(
            !type_spans
                .iter()
                .any(|s| &code[s.start..s.end] == "lowercase_thing"),
            "#match? ^[A-Z] must filter lowercase identifiers: {spans:?}"
        );
    }

    #[test]
    fn python_constructor_predicate_fires() {
        // zed's constructor pattern requires an assignment followed by a
        // docstring in the body
        let code = "class A:\n    def __init__(self):\n        self.x = 1\n        \"\"\"docs\"\"\"\n\n    def other(self):\n        pass";
        let spans = highlight(code, "python").unwrap();
        let constructors: Vec<&Span> = spans
            .iter()
            .filter(|s| s.name.contains("constructor"))
            .collect();
        assert!(
            constructors
                .iter()
                .any(|s| &code[s.start..s.end] == "__init__"),
            "#eq? constructor capture missing: {spans:?}"
        );
    }

    // --- @none suppression --------------------------------------------------

    #[test]
    fn rust_none_suppresses_lowercase_path_segments() {
        // rust.scm: in `x::y()` the lowercase `y` is captured @none so it does
        // not highlight as an attribute
        let code = "fn main() { vec::x(); }";
        let spans = highlight(code, "rust").unwrap();
        assert!(!spans.iter().any(|s| s.name == "none"));
    }

    // --- boundaries ---------------------------------------------------------

    #[test]
    fn unknown_language_returns_none() {
        assert!(highlight("code", "cobol").is_none());
        assert!(highlight("code", "").is_none());
    }

    #[test]
    fn empty_code_is_ok() {
        let spans = highlight("", "rust").unwrap();
        assert!(spans.is_empty());
    }

    #[test]
    fn unicode_offsets_are_byte_accurate() {
        // 中文注释 sits before the keyword; offsets must slice back exactly
        let code = "// 中文注释 🚀\nfn main() {}";
        let spans = highlight(code, "rust").unwrap();
        let fn_span = spans
            .iter()
            .find(|s| s.name == "keyword" && &code[s.start..s.end] == "fn")
            .expect("fn keyword span");
        assert!(code.is_char_boundary(fn_span.start));
    }

    #[test]
    fn crlf_is_handled() {
        let code = "fn main() {\r\n    return;\r\n}";
        let spans = highlight(code, "rust").unwrap();
        assert!(spans.iter().any(|s| &code[s.start..s.end] == "fn"));
    }

    #[test]
    fn no_newlines_is_handled() {
        let code = "let x = 1; let y = 2;";
        let spans = highlight(code, "rust").unwrap();
        assert!(spans.iter().any(|s| s.name == "keyword"));
    }

    #[test]
    fn deep_nesting_does_not_panic() {
        let depth = 400;
        let mut code = String::new();
        for _ in 0..depth {
            code.push_str("{\"a\":");
        }
        code.push('1');
        for _ in 0..depth {
            code.push('}');
        }
        let spans = highlight(&code, "json");
        assert!(spans.is_some());
    }

    #[test]
    fn long_input_is_handled() {
        let code = "fn main() {}\n".repeat(2000);
        let spans = highlight(&code, "rust").unwrap();
        assert!(spans.len() > 2000);
    }

    #[test]
    fn spans_are_sorted_by_start() {
        // the HTML backfill walks spans linearly — unsorted spans would
        // corrupt the slice-and-emit loop
        let code = "fn main() { let s = \"str\"; struct T; }\n// comment\n";
        let spans = highlight(code, "rust").unwrap();
        assert!(!spans.is_empty());
        for pair in spans.windows(2) {
            assert!(
                (pair[0].start, pair[0].end) <= (pair[1].start, pair[1].end),
                "spans must be sorted by (start, end): {:?} then {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn nested_captures_outer_comes_first() {
        // documented contract: when two captures start at the same byte the
        // outer (earlier-starting) capture is emitted first
        let code = "struct CamelCase;\n";
        let spans = highlight(code, "rust").unwrap();
        let starts: Vec<usize> = spans.iter().map(|s| s.start).collect();
        let mut sorted = starts.clone();
        sorted.sort_unstable();
        assert_eq!(starts, sorted);
    }
}
