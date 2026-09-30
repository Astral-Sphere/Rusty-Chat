//! Message-tree helpers for the chat blob (pure functions, host-testable).
//!
//! The blob's `history` shape (open-webui compatible):
//! `{ "messages": { id: { id, parentId, childrenIds[], role, content, … } },
//!    "currentId": id }`. Display shows the ancestor path of `currentId`;
//! siblings of a message are the parent's `childrenIds` in order.
//!
//! 覆盖矩阵（tests below）:
//! ✅ active_path：currentId 向上回溯 → 根到叶顺序；parentId 缺失/为 null
//!    的根消息；环引用防护（visited 集合）；currentId 缺失/为空 → 空 path
//! ✅ siblings_of：有 parent → parent.childrenIds 顺序；根消息 → 所有
//!    parentId 为 null 的消息按 timestamp 排序；消息不存在 → 空
//! ✅ edit path：编辑重发所需的 messages 数组（祖先链 + 编辑后内容）
//! ⛔ 刻意不覆盖：树写操作（后端 upsert_message_to_history 维护
//!    childrenIds/currentId，契约测试覆盖）；UI 状态

use serde_json::Value;

fn messages_of(history: &Value) -> &serde_json::Map<String, Value> {
    static EMPTY: std::sync::OnceLock<serde_json::Map<String, Value>> = std::sync::OnceLock::new();
    history
        .get("messages")
        .and_then(Value::as_object)
        .unwrap_or_else(|| EMPTY.get_or_init(Default::default))
}

fn parent_id(message: &Value) -> Option<&str> {
    message.get("parentId").and_then(Value::as_str)
}

/// The root→leaf chain of message ids ending at `history.currentId`.
/// Cycles are cut at the first revisit.
pub fn active_path(history: &Value) -> Vec<String> {
    let current = history
        .get("currentId")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    let Some(mut cursor) = current.map(str::to_string) else {
        return Vec::new();
    };
    let messages = messages_of(history);
    let mut chain = Vec::new();
    let mut visited = std::collections::HashSet::new();
    loop {
        if !visited.insert(cursor.clone()) {
            break;
        }
        let Some(message) = messages.get(&cursor) else {
            break;
        };
        chain.push(cursor.clone());
        match parent_id(message) {
            Some(parent) if !parent.is_empty() => cursor = parent.to_string(),
            _ => break,
        }
    }
    chain.reverse();
    chain
}

/// Sibling message ids of `message_id` (its parent's childrenIds, or all
/// root messages in timestamp order for parentId-less messages).
pub fn siblings_of(history: &Value, message_id: &str) -> Vec<String> {
    let messages = messages_of(history);
    let Some(message) = messages.get(message_id) else {
        return Vec::new();
    };
    if let Some(parent) = parent_id(message).filter(|p| !p.is_empty()) {
        return messages
            .get(parent)
            .and_then(|p| p.get("childrenIds"))
            .and_then(Value::as_array)
            .map(|children| {
                children
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
    }
    // root message: siblings are all roots, oldest first
    let mut roots: Vec<(i64, String)> = messages
        .iter()
        .filter(|(_, m)| parent_id(m).filter(|p| !p.is_empty()).is_none())
        .map(|(id, m)| {
            let ts = m.get("timestamp").and_then(Value::as_i64).unwrap_or(0);
            (ts, id.clone())
        })
        .collect();
    roots.sort_by_key(|(ts, id)| (*ts, id.clone()));
    roots.into_iter().map(|(_, id)| id).collect()
}

/// (index, count) of `message_id` among its siblings for the ‹ › switcher.
pub fn sibling_position(history: &Value, message_id: &str) -> (usize, usize) {
    let siblings = siblings_of(history, message_id);
    let index = siblings
        .iter()
        .position(|id| id == message_id)
        .map_or(0, |i| i + 1);
    (index, siblings.len())
}

/// The last descendant of `message_id` following the youngest-child chain —
/// branch navigation lands on the leaf so the whole branch shows
/// (open-webui Messages.svelte showPrevious/NextMessage semantics).
pub fn leaf_descendant(history: &Value, message_id: &str) -> String {
    let messages = messages_of(history);
    let mut current = message_id.to_string();
    loop {
        let Some(next) = messages
            .get(&current)
            .and_then(|m| m.get("childrenIds"))
            .and_then(Value::as_array)
            .and_then(|children| children.last())
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            return current;
        };
        if next == current {
            return current;
        }
        current = next;
    }
}

fn messages_mut(history: &mut Value) -> &mut serde_json::Map<String, Value> {
    if !history.is_object() {
        *history = serde_json::json!({"messages": {}, "currentId": null});
    }
    let obj = history.as_object_mut().expect("history is object");
    let messages = obj
        .entry("messages")
        .or_insert_with(|| Value::Object(Default::default()));
    if !messages.is_object() {
        *messages = Value::Object(Default::default());
    }
    messages.as_object_mut().expect("messages is object")
}

/// Local tree mutation mirroring what the server's
/// `upsert_message_to_history` does for a new node (and what open-webui's
/// editMessage/createMessagePair do client-side): insert the node under
/// `parent_id`, append it to the parent's `childrenIds`, and move
/// `currentId` to it. Keeps the local view consistent before the server
/// round-trip completes.
pub fn attach_message(history: &mut Value, node: Value, parent_id: Option<&str>) {
    let node_id = node
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if node_id.is_empty() {
        return;
    }
    let messages = messages_mut(history);
    if let Some(parent) = parent_id.filter(|p| !p.is_empty())
        && let Some(parent_node) = messages.get_mut(parent)
    {
        let children = parent_node.as_object_mut().map(|o| {
            o.entry("childrenIds")
                .or_insert_with(|| Value::Array(vec![]))
        });
        if let Some(Value::Array(children)) = children {
            let already = children
                .iter()
                .any(|c| c.as_str() == Some(node_id.as_str()));
            if !already {
                children.push(Value::String(node_id.clone()));
            }
        }
    }
    messages.insert(node_id.clone(), node);
    history
        .as_object_mut()
        .expect("history is object")
        .insert("currentId".into(), Value::String(node_id));
}

/// Convenience: attach a user message node.
pub fn attach_user_message(
    history: &mut Value,
    parent_id: Option<&str>,
    user_id: &str,
    content: &str,
    timestamp: i64,
) {
    attach_message(
        history,
        serde_json::json!({
            "id": user_id, "parentId": parent_id, "childrenIds": [],
            "role": "user", "content": content, "timestamp": timestamp,
        }),
        parent_id,
    );
}

/// Convenience: attach an assistant placeholder node (done=false). `model`
/// records the requesting model on the node so the UI can label the response
/// immediately (the server persists the same field).
pub fn attach_assistant_placeholder(
    history: &mut Value,
    parent_id: &str,
    assistant_id: &str,
    timestamp: i64,
    model: &str,
) {
    attach_message(
        history,
        serde_json::json!({
            "id": assistant_id, "parentId": parent_id, "childrenIds": [],
            "role": "assistant", "content": "", "done": false,
            "model": model, "timestamp": timestamp,
        }),
        Some(parent_id),
    );
}

/// The OpenAI `messages` array for regenerating from an edited user message:
/// the contents of its ancestor chain plus the edited content itself.
/// `history_messages` is the blob's messages map; `edited_parent_id` is the
/// parentId of the message being edited ("" / null for a root edit).
pub fn messages_for_regeneration(
    history: &Value,
    edited_parent_id: Option<&str>,
    edited_content: &str,
) -> Vec<Value> {
    let messages = messages_of(history);
    let mut chain = Vec::new();
    let mut visited = std::collections::HashSet::new();
    let mut cursor = edited_parent_id
        .filter(|p| !p.is_empty())
        .map(str::to_string);
    while let Some(id) = cursor {
        if !visited.insert(id.clone()) {
            break;
        }
        let Some(message) = messages.get(&id) else {
            break;
        };
        chain.push(json_message(message));
        cursor = parent_id(message)
            .filter(|p| !p.is_empty())
            .map(str::to_string);
    }
    chain.reverse();
    chain.push(serde_json::json!({"role": "user", "content": edited_content}));
    chain
}

fn json_message(message: &Value) -> Value {
    serde_json::json!({
        "role": message.get("role").and_then(Value::as_str).unwrap_or("user"),
        "content": message.get("content").cloned().unwrap_or(Value::Null),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn linear_history() -> Value {
        json!({
            "currentId": "a2",
            "messages": {
                "u1": {"id": "u1", "parentId": null, "childrenIds": ["a1"], "role": "user", "content": "q1", "timestamp": 1},
                "a1": {"id": "a1", "parentId": "u1", "childrenIds": ["u2"], "role": "assistant", "content": "r1", "timestamp": 2},
                "u2": {"id": "u2", "parentId": "a1", "childrenIds": ["a2"], "role": "user", "content": "q2", "timestamp": 3},
                "a2": {"id": "a2", "parentId": "u2", "childrenIds": [], "role": "assistant", "content": "r2", "timestamp": 4},
            }
        })
    }

    #[test]
    fn active_path_walks_root_to_leaf() {
        assert_eq!(active_path(&linear_history()), vec!["u1", "a1", "u2", "a2"]);
    }

    #[test]
    fn active_path_missing_or_empty_current_id() {
        assert!(active_path(&json!({})).is_empty());
        assert!(active_path(&json!({"currentId": "", "messages": {}})).is_empty());
        // dangling currentId terminates gracefully
        let history = json!({"currentId": "ghost", "messages": {}});
        assert!(active_path(&history).is_empty());
    }

    #[test]
    fn active_path_cuts_cycles() {
        let history = json!({
            "currentId": "u1",
            "messages": {
                "u1": {"id": "u1", "parentId": "a1", "childrenIds": [], "role": "user", "content": "x"},
                "a1": {"id": "a1", "parentId": "u1", "childrenIds": [], "role": "assistant", "content": "y"},
            }
        });
        // u1 → a1 → u1 (cycle) → stop; path is reversed and cut
        let path = active_path(&history);
        assert_eq!(path, vec!["a1", "u1"]);
    }

    #[test]
    fn siblings_via_parent_children_order() {
        let mut history = linear_history();
        // branch: u2 gets a second child
        history["messages"]["u2"]["childrenIds"] = json!(["a2", "a3"]);
        history["messages"]["a3"] = json!({"id": "a3", "parentId": "u2", "childrenIds": [], "role": "assistant", "content": "r3", "timestamp": 5});
        assert_eq!(siblings_of(&history, "a2"), vec!["a2", "a3"]);
        assert_eq!(sibling_position(&history, "a2"), (1, 2));
        assert_eq!(sibling_position(&history, "a3"), (2, 2));
    }

    #[test]
    fn siblings_of_roots_order_by_timestamp() {
        let history = json!({
            "currentId": "u1",
            "messages": {
                "u1": {"id": "u1", "parentId": null, "childrenIds": ["a1"], "role": "user", "content": "q1", "timestamp": 10},
                "u0": {"id": "u0", "parentId": null, "childrenIds": [], "role": "user", "content": "q0", "timestamp": 5},
                "a1": {"id": "a1", "parentId": "u1", "childrenIds": [], "role": "assistant", "content": "r1", "timestamp": 11},
            }
        });
        assert_eq!(siblings_of(&history, "u1"), vec!["u0", "u1"]);
        assert_eq!(sibling_position(&history, "u1"), (2, 2));
    }

    #[test]
    fn siblings_of_unknown_message_is_empty() {
        assert!(siblings_of(&linear_history(), "ghost").is_empty());
        assert_eq!(sibling_position(&linear_history(), "ghost"), (0, 0));
    }

    #[test]
    fn leaf_descendant_follows_youngest_children() {
        // u1 → a1 → u2 → a2: leaf of u1 (and of a2 itself) is a2
        let history = linear_history();
        assert_eq!(leaf_descendant(&history, "u1"), "a2");
        assert_eq!(leaf_descendant(&history, "a1"), "a2");
        assert_eq!(leaf_descendant(&history, "a2"), "a2");
        assert_eq!(leaf_descendant(&history, "ghost"), "ghost");
    }

    #[test]
    fn leaf_descendant_picks_last_branch_child() {
        let mut history = linear_history();
        history["messages"]["u2"]["childrenIds"] = json!(["a2", "a3"]);
        history["messages"]["a3"] = json!({"id": "a3", "parentId": "u2", "childrenIds": [], "role": "assistant", "content": "r3", "timestamp": 5});
        assert_eq!(leaf_descendant(&history, "u1"), "a3");
    }

    // --- local tree mutation (attach) ---------------------------------------

    #[test]
    fn attach_user_on_empty_history_creates_root_and_current() {
        let mut history = Value::Null;
        attach_user_message(&mut history, None, "u1", "hello", 1);
        attach_assistant_placeholder(&mut history, "u1", "a1", 2, "m1");
        assert_eq!(active_path(&history), vec!["u1", "a1"]);
        assert_eq!(history["messages"]["u1"]["childrenIds"], json!(["a1"]));
        assert_eq!(history["messages"]["a1"]["done"], json!(false));
        assert_eq!(history["currentId"], json!("a1"));
    }

    #[test]
    fn attach_edit_creates_sibling_and_moves_current_in_place() {
        // the open-webui edit flow: branch off u1 while currentId is a2
        let mut history = linear_history();
        attach_user_message(&mut history, None, "u1-edited", "q1 edited", 9);
        attach_assistant_placeholder(&mut history, "u1-edited", "a1-edited", 10, "m2");
        // new branch is the active path…
        assert_eq!(active_path(&history), vec!["u1-edited", "a1-edited"]);
        // …the old branch is intact…
        assert_eq!(history["messages"]["u1"]["childrenIds"], json!(["a1"]));
        assert_eq!(history["messages"]["a2"]["content"], json!("r2"));
        // …and both roots are switchable siblings
        assert_eq!(siblings_of(&history, "u1-edited"), vec!["u1", "u1-edited"]);
        assert_eq!(sibling_position(&history, "u1-edited"), (2, 2));
    }

    #[test]
    fn attach_follow_up_extends_active_leaf() {
        let mut history = linear_history();
        // currentId = a2 → follow-up user message parents onto it
        let current = history["currentId"].as_str().unwrap().to_string();
        attach_user_message(&mut history, Some(&current), "u3", "next question", 9);
        attach_assistant_placeholder(&mut history, "u3", "a3", 10, "m3");
        assert_eq!(
            active_path(&history),
            vec!["u1", "a1", "u2", "a2", "u3", "a3"]
        );
        assert_eq!(history["messages"]["a2"]["childrenIds"], json!(["u3"]));
    }

    #[test]
    fn attach_twice_does_not_duplicate_child_link() {
        let mut history = Value::Null;
        attach_user_message(&mut history, None, "u1", "hello", 1);
        attach_user_message(&mut history, None, "u1", "hello", 1);
        assert_eq!(history["messages"].as_object().unwrap().len(), 1);
    }

    #[test]
    fn messages_for_regeneration_walks_ancestors() {
        let history = linear_history();
        // edit u2 (parent a1): chain u1, a1 + edited content
        let messages = messages_for_regeneration(&history, Some("a1"), "q2 edited");
        assert_eq!(
            messages,
            vec![
                json!({"role": "user", "content": "q1"}),
                json!({"role": "assistant", "content": "r1"}),
                json!({"role": "user", "content": "q2 edited"}),
            ]
        );
        // root edit: only the edited message
        let messages = messages_for_regeneration(&history, None, "q1 edited");
        assert_eq!(
            messages,
            vec![json!({"role": "user", "content": "q1 edited"})]
        );
    }
}
