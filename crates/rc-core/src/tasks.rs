//! Task-model prompt templates — a 1:1 port of the title-generation path
//! from open-webui `utils/task.py` (`replace_prompt_variable`,
//! `replace_messages_variable`, `truncate_content`) plus the title
//! post-processing from `middleware.py` (`extract_title` and the 100-char
//! fallback truncation). Pure functions over `serde_json::Value` message
//! arrays so the router handler and the background title task share one
//! implementation.
//!
//! 覆盖矩阵（tests below）:
//! ✅ `{{prompt}}` / `{{prompt:start:n}}` / `{{prompt:end:n}}` /
//!    `{{prompt:middletruncate:n}}`，大小写不敏感（Python `(?i)` 等价）
//! ✅ `{{MESSAGES}}` / `{{MESSAGES|mode:count}}` / `{{MESSAGES:START:n}}` /
//!    `{{MESSAGES:END:n}}` / `{{MESSAGES:MIDDLETRUNCATE:n}}`（含过滤器；
//!    MIDDLETRUNCATE 的奇偶取半语义与 Python 逐行对齐）
//! ✅ 消息 content 为字符串或 `[{type:"text",text:…}]` 数组两种形状
//! ✅ `USER:`/`ASSISTANT:` 大写角色前缀与 "\n" 连接；无内容消息输出
//!    "None"（Python f-string 对 None 的行为）
//! ✅ extract_title：JSON 对象切片、缺 `title` 键 / 空串 / 解析失败 /
//!    无花括号四种分支的回退链
//! ✅ 回退标题 100 字符截断 + "..."（Python `[:100] + '...'`）
//! ✅ Unicode/emoji 计数按码点（与 Python str 切片一致）
//! ⛔ 刻意不覆盖：`{{USER_*}}`/`{{CURRENT_*}}` 用户变量替换（聊天中间件
//!    的 prompt_template，后续里程碑）、output items 文本提取（M6）、
//!    `title_string` 的 reasoning_content 来源选择（调用方职责）

use serde_json::Value;

/// open-webui `config.py` DEFAULT_TITLE_GENERATION_PROMPT_TEMPLATE, verbatim.
pub const DEFAULT_TITLE_GENERATION_PROMPT_TEMPLATE: &str = "### Task:
Generate a concise title summarizing the chat history.
### Guidelines:
- The title should clearly represent the main theme or subject of the conversation.
- Keep it short: 2-4 words is best.
- Do not use emojis, quotation marks, or special formatting.
- Write the title in the chat's primary language; default to English if multilingual.
- Prioritize accuracy over creativity.
- Your entire response must consist solely of the JSON object, without any introductory or concluding text.
- The output must be a single, raw JSON object, without any markdown code fences or other encapsulating text.
- Ensure no conversational text, affirmations, or explanations precede or follow the raw JSON output, as this will cause direct parsing failure.
### Output:
JSON format: { \"title\": \"your concise title here\" }
### Examples:
- { \"title\": \"Stock Trends\" },
- { \"title\": \"Chocolate Chip Cookies\" },
- { \"title\": \"Music Streaming\" },
- { \"title\": \"Remote Work\" }
### Chat History:
<chat_history>
{{MESSAGES:END:2}}
</chat_history>";

/// Text content of a message (Python `get_content_from_message`): string
/// content, or the first `{type: "text", text: …}` part when content is a
/// part array. `None` mirrors Python returning None.
pub fn message_content(message: &Value) -> Option<String> {
    let content = message.get("content")?;
    match content {
        Value::String(s) => Some(s.clone()),
        Value::Array(parts) => parts.iter().find_map(|p| {
            if p.get("type").and_then(Value::as_str) == Some("text") {
                p.get("text").and_then(Value::as_str).map(str::to_string)
            } else {
                None
            }
        }),
        _ => None,
    }
}

/// Last user-message content (Python `get_last_user_message`).
pub fn last_user_message(messages: &[Value]) -> Option<String> {
    messages
        .iter()
        .rev()
        .find(|m| m.get("role").and_then(Value::as_str) == Some("user"))
        .and_then(message_content)
}

/// Python `truncate_content` — code-point counts and slices, like Python str.
fn truncate_content(content: &str, max_chars: usize, mode: &str) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let chars: Vec<char> = content.chars().collect();
    if chars.len() <= max_chars {
        return content.to_string();
    }
    match mode {
        "start" => chars[..max_chars].iter().collect(),
        "end" => chars[chars.len() - max_chars..].iter().collect(),
        // middletruncate
        _ => {
            let half = max_chars / 2;
            let start: String = chars[..half].iter().collect();
            let end: String = chars[chars.len() - (max_chars - half)..].iter().collect();
            format!("{start}...{end}")
        }
    }
}

/// Applies a `mode:count` content filter to one message's text
/// (Python `apply_content_filter`, single-message form).
fn apply_filter(content: Option<String>, filter: &str) -> Option<String> {
    let (mode, count) = filter.split_once(':')?;
    let Ok(max) = count.parse::<usize>() else {
        return content;
    };
    let mode = mode.to_lowercase();
    if !matches!(mode.as_str(), "middletruncate" | "start" | "end") {
        return content;
    }
    content.map(|c| truncate_content(&c, max, &mode))
}

/// Python `replace_prompt_variable` — case-insensitive `{{prompt…}}` forms.
fn replace_prompt_variable(template: &str, prompt: Option<&str>) -> String {
    let re = regex::Regex::new(
        r"(?i)\{\{prompt(?::start:(?<start>\d+)|:end:(?<end>\d+)|:middletruncate:(?<mid>\d+))?\}\}",
    )
    .expect("static prompt regex");
    re.replace_all(template, |caps: &regex::Captures| {
        let Some(prompt) = prompt else {
            // Python raises when there is no user message; we degrade to an
            // empty substitution instead (documented divergence).
            return String::new();
        };
        if let Some(n) = caps.name("start") {
            let n: usize = n.as_str().parse().unwrap_or(0);
            return prompt.chars().take(n).collect();
        }
        if let Some(n) = caps.name("end") {
            let n: usize = n.as_str().parse().unwrap_or(0);
            let chars: Vec<char> = prompt.chars().collect();
            return chars[chars.len().saturating_sub(n)..].iter().collect();
        }
        if let Some(n) = caps.name("mid") {
            let n: usize = n.as_str().parse().unwrap_or(0);
            let chars: Vec<char> = prompt.chars().collect();
            if chars.len() <= n {
                return prompt.to_string();
            }
            let head: String = chars[..n.div_ceil(2)].iter().collect();
            let tail: String = chars[chars.len() - (n / 2)..].iter().collect();
            return format!("{head}...{tail}");
        }
        prompt.to_string()
    })
    .into_owned()
}

/// Python `replace_messages_variable` — the `{{MESSAGES…}}` family.
fn replace_messages_variable(template: &str, messages: &[Value]) -> String {
    let re = regex::Regex::new(concat!(
        r"\{\{MESSAGES(?:\|(?<bare>\w+:\d+))?\}\}",
        r"|\{\{MESSAGES:START:(?<start>\d+)(?:\|(?<startf>\w+:\d+))?\}\}",
        r"|\{\{MESSAGES:END:(?<end>\d+)(?:\|(?<endf>\w+:\d+))?\}\}",
        r"|\{\{MESSAGES:MIDDLETRUNCATE:(?<mid>\d+)(?:\|(?<midf>\w+:\d+))?\}\}",
    ))
    .expect("static messages regex");
    re.replace_all(template, |caps: &regex::Captures| {
        let selected: Vec<Value> = if let Some(n) = caps.name("start") {
            let n: usize = n.as_str().parse().unwrap_or(0);
            messages.iter().take(n).cloned().collect()
        } else if let Some(n) = caps.name("end") {
            let n: usize = n.as_str().parse().unwrap_or(0);
            messages
                .iter()
                .skip(messages.len().saturating_sub(n))
                .cloned()
                .collect()
        } else if let Some(n) = caps.name("mid") {
            let n: usize = n.as_str().parse().unwrap_or(0);
            if messages.len() <= n {
                messages.to_vec()
            } else {
                let half = n / 2;
                let tail = if n.is_multiple_of(2) { half } else { half + 1 };
                messages
                    .iter()
                    .take(half)
                    .cloned()
                    .collect::<Vec<_>>()
                    .into_iter()
                    .chain(messages.iter().skip(messages.len() - tail).cloned())
                    .collect()
            }
        } else {
            messages.to_vec()
        };
        let filter = ["bare", "startf", "endf", "midf"]
            .iter()
            .find_map(|g| caps.name(g))
            .map(|m| m.as_str().to_string());
        // the filter truncates each message's own content, then the block is
        // joined (Python apply_content_filter runs before get_messages_content)
        selected
            .iter()
            .map(|m| {
                let role = m
                    .get("role")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_uppercase();
                let content = message_content(m);
                let content = match &filter {
                    Some(f) => apply_filter(content, f),
                    None => content,
                };
                format!("{}: {}", role, content.unwrap_or_else(|| "None".into()))
            })
            .collect::<Vec<_>>()
            .join("\n")
    })
    .into_owned()
}

/// open-webui `title_generation_template` (user-variable pass excluded).
pub fn render_title_template(template: &str, messages: &[Value]) -> String {
    let prompt = last_user_message(messages);
    let template = replace_prompt_variable(template, prompt.as_deref());
    replace_messages_variable(&template, messages)
}

/// middleware.py title post-processing: slice the first `{`..last `}` of the
/// model response, parse it as JSON and read `title`. Falls back through
/// `user_message` (missing key) and `first_message_content` (parse failure /
/// empty title), mirroring `json.loads(...).get('title', user_message)` plus
/// `if not title: title = messages[0].get('content', user_message)`.
pub fn extract_title(
    content: &str,
    first_message_content: Option<&str>,
    user_message: Option<&str>,
) -> String {
    let parsed = content
        .find('{')
        .zip(content.rfind('}'))
        .filter(|(start, end)| end >= start)
        .and_then(|(start, end)| serde_json::from_str::<Value>(&content[start..=end]).ok());

    match parsed {
        Some(v) => match v.get("title").and_then(Value::as_str) {
            Some(t) if !t.is_empty() => t.to_string(),
            // key present but empty → Python `.get('title', …)` yields "" which
            // is falsy → falls to messages[0].content
            Some(_) => first_message_content.unwrap_or_default().to_string(),
            // missing key → `.get('title', user_message)`
            None => user_message
                .or(first_message_content)
                .unwrap_or_default()
                .to_string(),
        },
        // parse failure (including missing/unbalanced braces) → title = ""
        None => first_message_content.unwrap_or_default().to_string(),
    }
}

/// middleware.py truncates the fallback user message at 100 chars:
/// `user_message[:100] + '...'`.
pub fn truncate_title_fallback(content: &str) -> String {
    let chars: Vec<char> = content.chars().collect();
    if chars.len() > 100 {
        let head: String = chars[..100].iter().collect();
        format!("{head}...")
    } else {
        content.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn msgs() -> Vec<Value> {
        vec![
            json!({"role": "user", "content": "What is the capital of France?"}),
            json!({"role": "assistant", "content": "Paris."}),
        ]
    }

    // --- prompt variable ---------------------------------------------------

    #[test]
    fn prompt_plain_and_case_insensitive() {
        let out = replace_prompt_variable("Q: {{prompt}}!", Some("hello"));
        assert_eq!(out, "Q: hello!");
        let out = replace_prompt_variable("Q: {{PROMPT}}", Some("hello"));
        assert_eq!(out, "Q: hello");
    }

    #[test]
    fn prompt_start_end_middletruncate() {
        let long = "0123456789abcdef";
        assert_eq!(
            replace_prompt_variable("{{prompt:start:4}}", Some(long)),
            "0123"
        );
        assert_eq!(
            replace_prompt_variable("{{prompt:end:4}}", Some(long)),
            "cdef"
        );
        // middletruncate: head = ceil(n/2), tail = n/2
        assert_eq!(
            replace_prompt_variable("{{prompt:middletruncate:8}}", Some(long)),
            "0123...cdef"
        );
        assert_eq!(
            replace_prompt_variable("{{prompt:middletruncate:16}}", Some(long)),
            long
        );
    }

    #[test]
    fn prompt_without_user_message_becomes_empty() {
        assert_eq!(replace_prompt_variable("a {{prompt}} b", None), "a  b");
    }

    // --- messages variable -------------------------------------------------

    #[test]
    fn messages_end_two_matches_default_template() {
        let out = render_title_template(DEFAULT_TITLE_GENERATION_PROMPT_TEMPLATE, &msgs());
        assert!(
            out.contains("USER: What is the capital of France?"),
            "{out}"
        );
        assert!(out.contains("ASSISTANT: Paris."), "{out}");
        assert!(!out.contains("{{MESSAGES"), "{out}");
    }

    #[test]
    fn messages_start_and_bare() {
        let m = msgs();
        let out = replace_messages_variable("{{MESSAGES:START:1}}", &m);
        assert_eq!(out, "USER: What is the capital of France?");
        let out = replace_messages_variable("{{MESSAGES}}", &m);
        assert!(out.contains("USER: ") && out.contains("ASSISTANT: "));
    }

    #[test]
    fn messages_middletruncate_even_and_odd() {
        let many: Vec<Value> = (0..6)
            .map(|i| json!({"role": "user", "content": format!("m{i}")}))
            .collect();
        // even n: half + half
        let out = replace_messages_variable("{{MESSAGES:MIDDLETRUNCATE:4}}", &many);
        assert_eq!(out, "USER: m0\nUSER: m1\nUSER: m4\nUSER: m5");
        // odd n: half + half+1
        let out = replace_messages_variable("{{MESSAGES:MIDDLETRUNCATE:3}}", &many);
        assert_eq!(out, "USER: m0\nUSER: m4\nUSER: m5");
    }

    #[test]
    fn messages_content_filter() {
        let m = vec![json!({"role": "user", "content": "abcdefghij"})];
        let out = replace_messages_variable("{{MESSAGES:START:1|start:4}}", &m);
        assert_eq!(out, "USER: abcd");
        // invalid filter → the Python regex matches nothing → template kept
        let out = replace_messages_variable("{{MESSAGES|bogus}}", &m);
        assert_eq!(out, "{{MESSAGES|bogus}}");
    }

    #[test]
    fn messages_with_part_array_content() {
        let m = vec![json!({"role": "user", "content": [
            {"type": "image_url", "image_url": {"url": "x"}},
            {"type": "text", "text": "describe this"}
        ]})];
        let out = replace_messages_variable("{{MESSAGES}}", &m);
        assert_eq!(out, "USER: describe this");
    }

    #[test]
    fn messages_without_content_render_none() {
        let m = vec![json!({"role": "assistant"})];
        let out = replace_messages_variable("{{MESSAGES}}", &m);
        assert_eq!(out, "ASSISTANT: None");
    }

    // --- extract_title -------------------------------------------------------

    #[test]
    fn extract_title_json_object() {
        let out = extract_title(
            "Sure! {\"title\": \"Capital of France\"} done",
            Some("first"),
            Some("What is the capital of France?"),
        );
        assert_eq!(out, "Capital of France");
    }

    #[test]
    fn extract_title_missing_key_falls_back_to_user_message() {
        let out = extract_title("{\"reason\": \"none\"}", Some("first"), Some("user asked"));
        assert_eq!(out, "user asked");
    }

    #[test]
    fn extract_title_empty_title_falls_back_to_first_message() {
        let out = extract_title("{\"title\": \"\"}", Some("first"), Some("user asked"));
        assert_eq!(out, "first");
    }

    #[test]
    fn extract_title_parse_failure_falls_back_to_first_message() {
        let out = extract_title("no json at all", Some("first"), Some("user asked"));
        assert_eq!(out, "first");
        let out = extract_title("{unclosed", Some("first"), Some("user asked"));
        assert_eq!(out, "first");
    }

    #[test]
    fn extract_title_no_fallbacks_available() {
        assert_eq!(extract_title("nothing", None, None), "");
    }

    // --- fallback truncation -------------------------------------------------

    #[test]
    fn truncate_title_fallback_at_100_chars() {
        let short = "short";
        assert_eq!(truncate_title_fallback(short), short);
        let long: String = "x".repeat(150);
        let out = truncate_title_fallback(&long);
        assert_eq!(out.len(), 103); // 100 ASCII x + "..."
        assert!(out.ends_with("..."));
    }

    #[test]
    fn truncate_title_fallback_counts_code_points() {
        // 150 CJK chars → 100 kept (200 bytes) + "..."
        let long = "汉".repeat(150);
        let out = truncate_title_fallback(&long);
        assert_eq!(out.chars().count(), 103);
        assert!(out.starts_with("汉汉汉"));
    }
}
