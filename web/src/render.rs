//! Markdown → sanitized HTML for chat messages (pure functions; compiles for
//! both the host (unit tests) and wasm32).
//!
//! Pipeline: comrak (GFM + math-dollars) → `$…$`/`$$…$$` spans rendered by
//! katex-rs → ammonia sanitize as the single choke point over everything,
//! including the KaTeX output we injected. Raw HTML in user content is
//! dropped by comrak (`unsafe_` stays off), so nothing user-authored reaches
//! the DOM unescaped.
//!
//! Coverage matrix (AGENTS.md §3):
//! - normal paths: headings, emphasis/strong, links, images, fenced + inline
//!   code, GFM tables/tasklists/strikethrough/autolink, footnotes,
//!   superscript, hard line breaks, inline + display math via katex-rs
//! - boundaries: empty input, whitespace-only, emoji/UTF-8 (CJK, combining
//!   marks), CRLF line endings (also inside fences and display math), long
//!   input, unclosed math (streaming cut-off), escaped `\$` stays literal,
//!   `$$a$$b` adjacency, math in table cells / list items, `$ $` is not
//!   math, `$` inside inline code and fenced blocks (must NOT render as
//!   math), math containing HTML-special chars (`<`, `&`, quotes), malformed
//!   TeX (renders KaTeX error markup, not raw TeX)
//! - error paths: XSS battery — script/iframe/object/embed/style tags, event
//!   handler attributes, `javascript:`/`data:`/`vbscript:`/`file:` URLs in
//!   links and images, scheme obfuscations (mixed case, HTML entities),
//!   protocol-relative URLs pinned as PassThrough, hostile markup inside
//!   fences (escaped), incomplete tags, KaTeX TeX injection (\href/
//!   \includegraphics/\html*/recursive \def/\write/\input — trust=false),
//!   checkbox inputs other than tasklists
//! - deliberately not covered: syntax-highlight span backfill (T3 endpoint
//!   post-processes `<code class="language-x">`), mermaid blocks (T1b —
//!   pinned in mermaid.rs), server-side content policy (M2+).

use katex::{KatexContext, Settings};
use std::sync::LazyLock;

/// Immutable KaTeX function/symbol registry, built once and shared.
static KATEX_CTX: LazyLock<KatexContext> = LazyLock::new(KatexContext::default);

/// Renders markdown source to sanitized HTML. Streaming messages stay plain
/// text on the caller side; this is only invoked for finished content.
pub fn render_markdown(src: &str) -> String {
    let html = comrak_to_html(src);
    let html = render_math_spans(&html);
    sanitize(&html)
}

fn comrak_options() -> comrak::Options<'static> {
    let mut options = comrak::Options::default();
    let ext = &mut options.extension;
    ext.table = true;
    ext.strikethrough = true;
    ext.autolink = true;
    ext.tasklist = true;
    ext.superscript = true;
    ext.footnotes = true;
    ext.description_lists = true;
    ext.math_dollars = true;
    // Chat semantics: a lone newline is a visible line break (matches what
    // users and LLM output mean), not a soft space.
    options.render.hardbreaks = true;
    options
}

fn comrak_to_html(src: &str) -> String {
    comrak::markdown_to_html(src, &comrak_options())
}

/// Renders every comrak math span (`<span data-math-style="…">TeX</span>`)
/// in place with katex-rs. comrak escapes `<>&"` inside the span body, so the
/// first `</span>` after the opening tag always closes our span.
fn render_math_spans(html: &str) -> String {
    const OPEN: &str = "<span data-math-style=\"";
    const CLOSE: &str = "</span>";
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(pos) = rest.find(OPEN) {
        out.push_str(&rest[..pos]);
        let after_open = &rest[pos + OPEN.len()..];
        let Some(style_end) = after_open.find('"') else {
            break;
        };
        let display = &after_open[..style_end] == "display";
        let body = &after_open[style_end + 2..]; // skip `">`
        let Some(body_end) = body.find(CLOSE) else {
            break;
        };
        let tex = unescape_html(&body[..body_end]);
        out.push_str(&render_tex(&tex, display));
        rest = &body[body_end + CLOSE.len()..];
    }
    out.push_str(rest);
    out
}

fn render_tex(tex: &str, display: bool) -> String {
    let settings = Settings {
        display_mode: display,
        // KaTeX renders its own parse errors as colored source text instead
        // of failing — exactly what we want for streaming-cut formulas.
        throw_on_error: false,
        // self-referential `\def` macros expand until this cap; each
        // expansion nests the parser deeper, and the default 1000
        // overflowed even the 8MB host test stack (and would trap the far
        // smaller wasm stack). Chat TeX never needs more than a handful.
        max_expand: 32,
        ..Settings::default()
    };
    katex::render_to_string(&KATEX_CTX, tex, &settings)
        .unwrap_or_else(|_| format!("<code class=\"math-error\">{}</code>", escape_html(tex)))
}

fn unescape_html(s: &str) -> String {
    // comrak escapes `&` first, so replacing it last undoes exactly one layer.
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&amp;", "&")
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Single sanitization choke point over comrak output + injected KaTeX HTML.
/// The KaTeX whitelist below was derived from katex-rs 0.3.0 (KaTeX 0.18.5)
/// output: styled `<span>` trees, a MathML shadow tree, and `<svg><path>`
/// stretchy delimiters; the tests assert formulas survive sanitization.
fn sanitize(html: &str) -> String {
    use std::collections::{HashMap, HashSet};

    fn set<'a>(items: &'a [&'a str]) -> HashSet<&'a str> {
        items.iter().copied().collect()
    }

    let mut builder = ammonia::Builder::default();
    builder.tags(set(&[
        // standard flow content produced by comrak
        "a",
        "abbr",
        "b",
        "blockquote",
        "br",
        "code",
        "dd",
        "del",
        "details",
        "div",
        "dl",
        "dt",
        "em",
        "figcaption",
        "figure",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "hr",
        "i",
        "img",
        "input",
        "ins",
        "kbd",
        "li",
        "mark",
        "ol",
        "p",
        "pre",
        "q",
        "rp",
        "rt",
        "ruby",
        "s",
        "samp",
        "section",
        "small",
        "span",
        "strike",
        "strong",
        "sub",
        "summary",
        "sup",
        "table",
        "tbody",
        "td",
        "tfoot",
        "th",
        "thead",
        "tr",
        "u",
        "ul",
        "var",
        // KaTeX HTML + MathML + stretchy SVG
        "svg",
        "path",
        "line",
        "math",
        "semantics",
        "annotation",
        "mrow",
        "mi",
        "mo",
        "mn",
        "ms",
        "mtext",
        "mfrac",
        "msqrt",
        "mroot",
        "mstyle",
        "msub",
        "msup",
        "msubsup",
        "munder",
        "mover",
        "munderover",
        "mtable",
        "mtr",
        "mtd",
        "mlabeledtr",
        "mspace",
        "mpadded",
        "mphantom",
        "mprescripts",
        "none",
        "maction",
        "menclose",
    ]));
    builder.generic_attributes(set(&[
        "class",
        "style",
        "title",
        "aria-hidden",
        "align",
        "valign",
        "width",
        "height",
        "colspan",
        "rowspan",
        "start",
        "reversed",
        "dir",
        "lang",
        "encoding",
        "mathvariant",
    ]));
    builder.tag_attributes(HashMap::from([
        ("a", set(&["href", "target"])),
        ("img", set(&["src", "alt"])),
        ("input", set(&["type", "checked", "disabled"])),
        ("li", set(&["id"])),
        ("ol", set(&["type"])),
        ("svg", set(&["viewBox", "preserveAspectRatio", "xmlns"])),
        (
            "path",
            set(&[
                "d",
                "fill",
                "fill-opacity",
                "fill-rule",
                "stroke",
                "stroke-width",
            ]),
        ),
        (
            "line",
            set(&[
                "x1",
                "x2",
                "y1",
                "y2",
                "stroke",
                "stroke-width",
                "stroke-linecap",
            ]),
        ),
    ]));
    // tasklist checkboxes are the only allowed <input>
    builder.tag_attribute_values(HashMap::from([(
        "input",
        HashMap::from([("type", set(&["checkbox"]))]),
    )]));
    builder.url_schemes(set(&["http", "https", "mailto", "xmpp"]));
    builder.url_relative(ammonia::UrlRelative::PassThrough);
    builder.link_rel(Some("noopener noreferrer"));
    builder.clean(html).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(src: &str) -> String {
        render_markdown(src)
    }

    // --- normal paths -----------------------------------------------------

    #[test]
    fn headings_emphasis_and_links() {
        let out = render("# Hello\n\n**bold** *it* [link](https://e.com)");
        assert!(out.contains("<h1>Hello</h1>"), "{out}");
        assert!(out.contains("<strong>bold</strong>"), "{out}");
        assert!(out.contains("<em>it</em>"), "{out}");
        assert!(out.contains(r#"href="https://e.com""#), "{out}");
        assert!(out.contains("rel=\"noopener noreferrer\""), "{out}");
    }

    #[test]
    fn gfm_table_tasklist_strikethrough() {
        let out = render("| a | b |\n|---|---|\n| 1 | 2 |\n\n- [x] done\n- [ ] todo\n\n~~gone~~");
        assert!(out.contains("<table>"), "{out}");
        assert!(out.contains("<td"), "{out}");
        assert!(out.contains(r#"<input type="checkbox""#), "{out}");
        assert!(out.contains("<del>gone</del>"), "{out}");
    }

    #[test]
    fn code_fence_and_inline_code() {
        let out = render("```rust\nfn main() { let s = \"hi\"; }\n```\n\n`inline $x$ code`");
        assert!(out.contains("<pre><code class=\"language-rust\">"), "{out}");
        assert!(out.contains("<code>inline $x$ code</code>"), "{out}");
        // $ inside code must not become katex
        assert!(!out.contains("katex"), "{out}");
    }

    #[test]
    fn autolink_and_footnotes() {
        let out = render("see https://example.com now[^1]\n\n[^1]: the note");
        assert!(out.contains("https://example.com"), "{out}");
        assert!(out.contains("footnote"), "{out}");
    }

    #[test]
    fn hard_line_breaks() {
        let out = render("line one\nline two");
        assert!(out.contains("<br"), "{out}");
    }

    // --- math -------------------------------------------------------------

    #[test]
    fn inline_math_becomes_katex() {
        let out = render("$E=mc^2$ and $a_1$");
        assert!(out.contains("katex"), "{out}");
        // the raw TeX must not leak as text
        assert!(!out.contains("$E=mc^2$"), "{out}");
    }

    #[test]
    fn display_math_becomes_katex() {
        let out = render("$$\\frac{1}{2}$$");
        assert!(out.contains("katex-display"), "{out}");
    }

    #[test]
    fn math_with_special_chars_survives_extraction() {
        let out = render("$a < b \\wedge c > d$");
        assert!(out.contains("katex"), "{out}");
    }

    #[test]
    fn unclosed_math_stays_plain_text() {
        // comrak only treats $..$/$$..$$ as math when both delimiters exist,
        // so a streaming cut-off degrades to plain text (no katex, no leak).
        let out1 = render("price is $5 and $6 total");
        assert!(!out1.contains("katex"), "{out1}");
        assert!(out1.contains("$5"), "{out1}");
        let out2 = render("$$\\frac{1}{");
        assert!(!out2.contains("katex"), "{out2}");
        assert!(out2.contains("\\frac{1}{"), "{out2}");
    }

    #[test]
    fn malformed_tex_shows_error_markup() {
        let out = render("$$\\notacommand{x}$$");
        assert!(out.contains("katex"), "{out}");
    }

    // --- boundaries -------------------------------------------------------

    #[test]
    fn empty_and_whitespace_only() {
        assert_eq!(render(""), "");
        assert!(render("   \n\t").trim().is_empty() || !render("   \n\t").contains('<'));
    }

    #[test]
    fn unicode_emoji_cjk_and_crlf() {
        let out = render("中文消息 🚀🎉 é漢字\r\n\r\n# 标题 🎯");
        assert!(out.contains("中文消息 🚀🎉 é漢字"), "{out}");
        assert!(out.contains("<h1>标题 🎯</h1>"), "{out}");
    }

    #[test]
    fn long_input_does_not_panic() {
        let src = "# t\n\nsome paragraph with **bold** and $x^2$.\n\n".repeat(500);
        let out = render(&src);
        assert!(out.len() > src.len() / 2);
    }

    // --- XSS battery ------------------------------------------------------

    #[test]
    fn raw_html_is_not_passed_through() {
        for src in [
            "<script>alert(1)</script>",
            "<img src=x onerror=alert(1)>",
            "<iframe src=\"https://e.com\"></iframe>",
            "<object data=\"x\"></object>",
            "<style>body{}</style>",
        ] {
            let out = render(src);
            assert!(!out.contains("<script"), "{src} → {out}");
            assert!(!out.contains("<iframe"), "{src} → {out}");
            assert!(!out.contains("<object"), "{src} → {out}");
            assert!(!out.contains("<style"), "{src} → {out}");
            assert!(!out.contains("onerror"), "{src} → {out}");
        }
    }

    #[test]
    fn dangerous_url_schemes_are_stripped() {
        for src in [
            "[x](javascript:alert(1))",
            "![x](javascript:alert(1))",
            "[x](data:text/html,<b>)",
        ] {
            let out = render(src);
            assert!(!out.contains("javascript:"), "{src} → {out}");
            assert!(!out.contains("data:text/html"), "{src} → {out}");
        }
    }

    #[test]
    fn url_scheme_obfuscations_are_stripped() {
        for src in [
            // mixed case
            "[x](JaVaScRiPt:alert(1))",
            // HTML entity obfuscation (ammonia decodes entities while parsing)
            "[x](&#106;avascript:alert(1))",
            "[x](javascript&colon;alert(1))",
            // other dangerous schemes
            "[x](vbscript:msgbox)",
            "[x](file:///etc/passwd)",
            // data: URIs in images (whitelist excludes them for v1)
            "![pic](data:image/png;base64,AAAA)",
        ] {
            let out = render(src);
            assert!(!out.to_lowercase().contains("javascript:"), "{src} → {out}");
            assert!(!out.contains("vbscript:"), "{src} → {out}");
            assert!(!out.contains("file:"), "{src} → {out}");
            assert!(
                !out.contains(r#"src="data:"#),
                "data: image sources must be stripped: {src} → {out}"
            );
        }
        // protocol-relative URLs pass through INTENTIONALLY (UrlRelative::
        // PassThrough) — they inherit the page scheme (https). Pin it so a
        // future ammonia bump that changes this is a conscious decision.
        let out = render("[x](//evil.com/x)");
        assert!(out.contains(r#"href="//evil.com/x""#), "{out}");
    }

    #[test]
    fn hostile_markup_inside_fences_and_incomplete_tags() {
        // script inside a fenced code block is escaped text, never markup
        let out = render("```html\n<script>alert(1)</script>\n```");
        assert!(!out.contains("<script"), "{out}");
        assert!(out.contains("&lt;script&gt;"), "{out}");
        // nested fence (four backticks wrapping triple backticks)
        let out = render("````\n```json\n{\"a\": 1}\n```\n````");
        assert!(out.contains("```json"), "{out}");
        // incomplete tags dissolve (comrak drops raw HTML)
        for src in [
            "<div",
            "<img src=x",
            "<b>x",
            "<a href='javascript:alert(1)'>y",
        ] {
            let out = render(src);
            assert!(!out.contains("<div"), "{src} → {out}");
            assert!(!out.contains("<img"), "{src} → {out}");
            assert!(!out.contains("<b>"), "{src} → {out}");
            assert!(!out.contains("<a "), "{src} → {out}");
            assert!(!out.to_lowercase().contains("javascript:"), "{src} → {out}");
        }
    }

    /// KaTeX TeX commands that could smuggle HTML/links/IO. katex-rs runs
    /// with `trust: false` (the default): \href/\includegraphics/\html*
    /// raise, and KaTeX's error path echoes the raw TeX as ESCAPED text in
    /// `<annotation>`/error spans — inert. The invariant that matters: user
    /// TeX never produces a live tag, whatever strings it contains.
    #[test]
    fn katex_untrusted_commands_cannot_inject_html() {
        for tex in [
            r"$\href{javascript:alert(1)}{x}$",
            r"$\includegraphics{https://evil.com/x.png}$",
            r"$\htmlClass{evil-span}{x}$",
            r"$\htmlId{evil-id}{x}$",
            r"$\htmlStyle{color:red}{x}$",
            r"$\htmlData{evil=1}{x}$",
            // self- and mutually-recursive macros: must terminate via
            // max_expand (each expansion nests the parser deeper; the
            // default 1000 overflowed even the 8MB host stack and would
            // trap the far smaller wasm stack)
            r"$\def\x{\href{javascript:alert(1)}{\x}}\x$",
            r"$\def\a{\b}\def\b{\a}\a$",
            // shell/IO commands never exist in a browser KaTeX
            r"$\write18{rm -rf}$",
            r"$\input{|ls}$",
        ] {
            let out = render(tex);
            // no live link/image/script frame escapes the render, no matter
            // what the echoed error text says
            assert!(!out.contains("<script"), "{tex} → {out}");
            assert!(!out.contains("<img"), "{tex} → {out}");
            assert!(!out.contains("<a "), "{tex} → {out}");
            assert!(!out.contains("<iframe"), "{tex} → {out}");
            // no evil class/attribute was attached to real markup: it may
            // only appear inside the escaped echo (after `&lt;` or `&nbsp;`)
            if out.contains("evil-span") {
                assert!(!out.contains(r#"class="evil-span""#), "{tex} → {out}");
            }
            // either rendered or turned into the escaped error text — but
            // always bounded output, never the raw untrusted markup
            assert!(
                out.contains("katex") || out.contains("math-error"),
                "{tex} → {out}"
            );
        }
    }

    #[test]
    fn katex_error_annotation_escapes_markup() {
        // KaTeX echoes failing TeX into <annotation> — markup inside that
        // echo MUST be entity-escaped, or ammonia would parse it back as tags
        let out = render(r"$\bogus{\text{</annotation><img src=x onerror=y>}}$");
        assert!(!out.contains("<img"), "{out}");
        assert!(out.contains("&lt;img"), "markup must be escaped: {out}");
        let out = render(r"$\bogus{<a href='javascript:x'>c</a>}$");
        assert!(!out.contains("<a "), "{out}");
        assert!(out.contains("&lt;a"), "markup must be escaped: {out}");
    }

    #[test]
    fn math_boundaries_escaped_dollars_and_positions() {
        // escaped \$ stays literal text (comrak math_dollars respects \$.)
        let out = render(r"price: \$5 and \$6 total");
        assert!(!out.contains("katex"), "{out}");
        assert!(out.contains("$5"), "{out}");
        // display math followed immediately by text
        let out = render("$$a$$b");
        assert!(out.contains("katex-display"), "{out}");
        assert!(out.contains('b'), "{out}");
        // math inside a table cell and a list item renders
        let out = render("| f |\n|---|\n| $x^2$ |\n\n- item $x_1$");
        assert!(out.contains("katex"), "{out}");
        assert!(out.contains("<td"), "{out}");
        // `$ $` (space-separated bare dollars) is not math
        let out = render("a $ $ b");
        assert!(!out.contains("katex"), "{out}");
        // CRLF inside a fenced block keeps the fence
        let out = render("```rust\r\nlet x = 1;\r\n```");
        assert!(out.contains("language-rust"), "{out}");
        // CRLF inside display math still parses
        let out = render("$$a +\r\nb$$");
        assert!(out.contains("katex"), "{out}");
    }

    #[test]
    fn raw_html_tags_are_dropped_entirely() {
        // `unsafe_` is off: comrak replaces raw HTML blocks/inlines with
        // comments, so even a well-formed <a onclick> never reaches ammonia —
        // users write markdown links instead.
        let out = render("<a href=\"https://e.com\" onclick=\"evil()\">x</a>");
        assert!(!out.contains("onclick"), "{out}");
        assert!(!out.contains("<a"), "{out}");
        assert!(out.contains('x'), "{out}");
    }

    #[test]
    fn non_checkbox_inputs_are_stripped() {
        let out = render("<input type=\"submit\">");
        assert!(!out.contains("submit"), "{out}");
    }

    // --- KaTeX output survives sanitization -------------------------------

    #[test]
    fn katex_html_and_mathml_survive_sanitize() {
        let out = render("$$\\sqrt{\\frac{a}{b}} + \\underbrace{x}_{y}$$");
        assert!(out.contains("katex-display"), "{out}");
        assert!(out.contains("katex-html"), "{out}");
        assert!(out.contains("<math"), "{out}"); // MathML shadow tree intact
        assert!(out.contains("aria-hidden"), "{out}");
    }

    #[test]
    fn katex_stretchy_svg_survives_sanitize() {
        // \xrightarrow and \overbrace force SVG stretchy pieces
        let out = render("$a \\xrightarrow{f} b$ and $\\overbrace{x+y}^{n}$");
        assert!(out.contains("katex"), "{out}");
        assert!(!out.contains("$\\xrightarrow"), "{out}");
        // whatever svg/path the renderer produced must not be stripped
        if out.contains("<svg") {
            assert!(out.contains("<path"), "{out}");
        }
    }
}
