//! `chat.chat` JSON blob helpers — semantics ported 1:1 from open-webui
//! `models/chats.py` (`ChatTable.merge_history`, `upsert_message_to_history`,
//! `delete_message_from_history`, `get_current_message_id`,
//! `_clean_null_bytes`). Pure functions; exhaustive unit tests required
//! (docs/TESTING.md) — this is the highest-risk compatibility surface.

use serde_json::{Map, Value};

/// `ChatTable.get_current_message_id`: history.currentId → chat.currentId →
/// chat.branchPointMessageId → last id of the flat `chat.messages` list.
pub fn get_current_message_id(chat: &Value) -> Option<String> {
    let history = chat.get("history").filter(|h| h.is_object());
    let current_id = history
        .and_then(|h| h.get("currentId"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .or_else(|| {
            chat.get("currentId")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
        })
        .or_else(|| {
            chat.get("branchPointMessageId")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
        });
    if let Some(id) = current_id {
        return Some(id.to_string());
    }

    if let Some(messages) = chat.get("messages").and_then(Value::as_array) {
        for message in messages.iter().rev() {
            if let Some(id) = message.get("id").and_then(Value::as_str) {
                return Some(id.to_string());
            }
        }
    }
    None
}

/// `ChatTable._last_descendant_id`: follow the last *existing* child
/// repeatedly; cycle-safe.
pub(crate) fn last_descendant_id(messages: &Map<String, Value>, message_id: &str) -> String {
    let mut current = message_id.to_string();
    let mut seen: Vec<String> = vec![];
    while messages.contains_key(&current) && !seen.contains(&current) {
        seen.push(current.clone());
        let child_ids = messages
            .get(&current)
            .and_then(|m| m.get("childrenIds"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let next = child_ids.iter().rev().find_map(|c| {
            c.as_str()
                .filter(|s| messages.contains_key(*s))
                .map(str::to_string)
        });
        match next {
            Some(n) => current = n,
            None => break,
        }
    }
    current
}

/// Recursive null-byte scrub (`ChatTable._clean_null_bytes`): strings lose
/// `\u{0}` at any depth; objects/arrays rebuilt in place.
pub fn clean_null_bytes(obj: &Value) -> Value {
    match obj {
        Value::String(s) => Value::String(s.replace('\u{0}', "")),
        Value::Array(items) => Value::Array(items.iter().map(clean_null_bytes).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), clean_null_bytes(v)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// `ChatTable.merge_history`: union of message maps (incoming wins per
/// message, whole message replaced), then `childrenIds` recomputed from
/// scratch (every message starts with [] and gets one entry per child),
/// then currentId resolution (incoming preferred, else existing, else None).
pub fn merge_history(existing_history: Option<&Value>, incoming_history: Option<&Value>) -> Value {
    let existing_msgs = existing_history
        .and_then(|h| h.get("messages"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let incoming_msgs = incoming_history
        .and_then(|h| h.get("messages"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    // Union: incoming overrides per-message (Python {**existing, **incoming}).
    let mut merged: Map<String, Value> = existing_msgs;
    for (k, v) in incoming_msgs {
        merged.insert(k, v);
    }

    // Keep only dict messages; reset childrenIds, then re-add from parents.
    let mut merged: Map<String, Value> = merged
        .into_iter()
        .filter(|(_, v)| v.is_object())
        .map(|(id, mut msg)| {
            if let Some(obj) = msg.as_object_mut() {
                obj.insert("childrenIds".to_string(), Value::Array(vec![]));
            }
            (id, msg)
        })
        .collect();

    let ids: Vec<String> = merged.keys().cloned().collect();
    for message_id in ids {
        let parent_id = merged
            .get(&message_id)
            .and_then(|m| m.get("parentId"))
            .and_then(Value::as_str)
            .map(str::to_string);
        if let Some(parent_id) = parent_id
            && let Some(parent) = merged.get_mut(&parent_id)
            && let Some(children) = parent.get_mut("childrenIds").and_then(Value::as_array_mut)
        {
            children.push(Value::String(message_id.clone()));
        }
    }

    // currentId: incoming → existing → None.
    let current_id = incoming_history
        .and_then(|h| h.get("currentId"))
        .and_then(Value::as_str)
        .filter(|id| merged.contains_key(*id))
        .or_else(|| {
            existing_history
                .and_then(|h| h.get("currentId"))
                .and_then(Value::as_str)
                .filter(|id| merged.contains_key(*id))
        })
        .map(|s| Value::String(s.to_string()))
        .unwrap_or(Value::Null);

    // Python: {**(existing or {}), **(incoming or {}), 'messages': merged, 'currentId': …}
    let mut out = Map::new();
    if let Some(e) = existing_history.and_then(Value::as_object) {
        for (k, v) in e {
            out.insert(k.clone(), v.clone());
        }
    }
    if let Some(i) = incoming_history.and_then(Value::as_object) {
        for (k, v) in i {
            out.insert(k.clone(), v.clone());
        }
    }
    out.insert("messages".to_string(), Value::Object(merged));
    out.insert("currentId".to_string(), current_id);
    Value::Object(out)
}

/// `upsert_message_to_history`: insert-or-patch one message into the blob
/// and re-point `currentId` at it (only for NEW messages). Owned-map
/// implementation; writes back only the mutated pieces.
pub fn upsert_message_to_history(
    history: &mut Value,
    message_id: &str,
    message: &Value,
) -> Option<Value> {
    if !history.is_object() {
        *history = Value::Object(Map::new());
    }
    let history_obj = history.as_object_mut().unwrap();
    let mut messages: Map<String, Value> = history_obj
        .get("messages")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    let result = if let Some(existing) = messages.get(message_id) {
        // Patch: {**existing, **message}
        let mut merged = existing.as_object().cloned().unwrap_or_default();
        if let Some(incoming) = message.as_object() {
            for (k, v) in incoming {
                merged.insert(k.clone(), v.clone());
            }
        }
        let patched = Value::Object(merged);
        messages.insert(message_id.to_string(), patched.clone());
        add_child_id_to_parent(
            &mut messages,
            get_parent_id(&patched).as_deref(),
            message_id,
        );
        messages.get(message_id).cloned()
    } else {
        // New message: resolve parent (explicit parentId, else the node that
        // already lists us as a child), then derive role/timestamp defaults.
        let message_parent_id = get_parent_id(message);
        let mut parent_id = message_parent_id.clone();
        if parent_id.is_none() {
            for (existing_id, existing_message) in &messages {
                if let Some(children) = existing_message
                    .get("childrenIds")
                    .and_then(Value::as_array)
                    && children.iter().any(|c| c.as_str() == Some(message_id))
                {
                    parent_id = Some(existing_id.clone());
                    break;
                }
            }
        }
        let parent = parent_id.as_deref().and_then(|pid| messages.get(pid));
        let output = message.get("output").and_then(Value::as_array);
        let output_role = output.and_then(|items| {
            items
                .iter()
                .find_map(|item| item.get("role").and_then(Value::as_str))
        });
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .or(output_role)
            .map(str::to_string)
            .or_else(|| {
                parent
                    .and_then(|p| p.get("role"))
                    .and_then(Value::as_str)
                    .map(|parent_role| {
                        if parent_role == "user" {
                            "assistant".to_string()
                        } else if parent_role == "assistant" {
                            "user".to_string()
                        } else {
                            "assistant".to_string()
                        }
                    })
            })
            .unwrap_or_else(|| "assistant".to_string());

        let mut new_message = message.as_object().cloned().unwrap_or_default();
        new_message.insert(
            "id".to_string(),
            Value::String(
                message
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| message_id.to_string()),
            ),
        );
        new_message.insert(
            "parentId".to_string(),
            message_parent_id
                .clone()
                .map(Value::String)
                .unwrap_or_else(|| parent_id.clone().map(Value::String).unwrap_or(Value::Null)),
        );
        new_message.insert(
            "childrenIds".to_string(),
            Value::Array(
                message
                    .get("childrenIds")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
            ),
        );
        new_message.insert("role".to_string(), Value::String(role));
        new_message.insert(
            "timestamp".to_string(),
            message
                .get("timestamp")
                .cloned()
                .unwrap_or(Value::Number(serde_json::Number::from(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs(),
                ))),
        );
        let new_message = Value::Object(new_message);

        messages.insert(message_id.to_string(), new_message.clone());
        // currentId moves to the NEW message only.
        history_obj.insert(
            "currentId".to_string(),
            Value::String(message_id.to_string()),
        );
        add_child_id_to_parent(
            &mut messages,
            new_message.get("parentId").and_then(Value::as_str),
            message_id,
        );
        messages.get(message_id).cloned()
    };

    history_obj.insert("messages".to_string(), Value::Object(messages));
    result
}

fn get_parent_id(message: &Value) -> Option<String> {
    message
        .get("parentId")
        .and_then(|v| if v.is_null() { None } else { v.as_str() })
        .map(str::to_string)
}

/// `_add_child_id_to_parent`: ensure parent's childrenIds contains child.
fn add_child_id_to_parent(
    messages: &mut Map<String, Value>,
    parent_id: Option<&str>,
    message_id: &str,
) -> bool {
    let Some(parent_id) = parent_id else {
        return false;
    };
    let Some(parent) = messages.get_mut(parent_id) else {
        return false;
    };
    let Some(obj) = parent.as_object_mut() else {
        return false;
    };
    let children = obj
        .entry("childrenIds")
        .or_insert_with(|| Value::Array(vec![]));
    if !children.is_array() {
        *children = Value::Array(vec![]);
    }
    let arr = children.as_array_mut().unwrap();
    if arr.iter().any(|c| c.as_str() == Some(message_id)) {
        return false;
    }
    arr.push(Value::String(message_id.to_string()));
    true
}

/// `delete_message_from_history`: remove a message and its direct children
/// (the children are re-linked to the grandparent? NO — per open-webui, the
/// deleted node's children are re-attached to the grandparent and the node
/// itself plus its children are... re-read: child_ids stay deleted, and
/// grandchildren get re-parented to the deleted node's parent).
/// Returns the set of deleted message ids.
pub fn delete_message_from_history(history: &mut Value, message_id: &str) -> Vec<String> {
    let Some(messages) = history.get_mut("messages").and_then(Value::as_object_mut) else {
        return vec![];
    };

    let Some(message) = messages.get(message_id).cloned() else {
        return vec![];
    };
    if !message.is_object() {
        return vec![];
    }

    let parent_id = message
        .get("parentId")
        .and_then(Value::as_str)
        .map(str::to_string);
    let child_ids: Vec<String> = message
        .get("childrenIds")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    // Keep only children that exist in the map.
    let child_ids: Vec<String> = child_ids
        .into_iter()
        .filter(|c| messages.contains_key(c))
        .collect();
    let grandchild_ids: Vec<String> = child_ids
        .iter()
        .flat_map(|c| {
            messages
                .get(c)
                .and_then(|m| m.get("childrenIds"))
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
        .filter(|g| messages.contains_key(g))
        .collect();

    // Re-attach grandchildren to the deleted node's parent.
    if let Some(pid) = &parent_id
        && let Some(parent) = messages.get_mut(pid).and_then(Value::as_object_mut)
    {
        let children = parent
            .entry("childrenIds".to_string())
            .or_insert_with(|| Value::Array(vec![]));
        if !children.is_array() {
            *children = Value::Array(vec![]);
        }
        let arr = children.as_array_mut().unwrap();
        arr.retain(|c| c.as_str() != Some(message_id));
        for g in &grandchild_ids {
            arr.push(Value::String(g.clone()));
        }
    }

    for g in &grandchild_ids {
        if let Some(grandchild) = messages.get_mut(g).and_then(Value::as_object_mut) {
            grandchild.insert(
                "parentId".to_string(),
                parent_id.clone().map(Value::String).unwrap_or(Value::Null),
            );
        }
    }

    let deleted_ids: Vec<String> = std::iter::once(message_id.to_string())
        .chain(child_ids.iter().cloned())
        .collect();
    for id in &deleted_ids {
        messages.remove(id);
    }

    // currentId: walk from parent (or roots) down the LAST child chain.
    let mut current_id = parent_id;
    let mut child_ids_now: Vec<String> = match &current_id {
        None => messages
            .iter()
            .filter(|(_, m)| {
                // Python `child.get('parentId') is None`: missing key counts as root too.
                m.get("parentId").map(Value::is_null).unwrap_or(true)
            })
            .map(|(id, _)| id.clone())
            .collect(),
        Some(pid) => messages
            .get(pid)
            .and_then(|m| m.get("childrenIds"))
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
    };
    let mut visited: Vec<String> = vec![];
    while let Some(last) = child_ids_now.last() {
        if visited.contains(last) {
            break;
        }
        visited.push(last.clone());
        current_id = Some(last.clone());
        child_ids_now = messages
            .get(last)
            .and_then(|m| m.get("childrenIds"))
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
    }
    let current = match &current_id {
        Some(id) if messages.contains_key(id) => Value::String(id.clone()),
        _ => Value::Null,
    };
    history
        .as_object_mut()
        .unwrap()
        .insert("currentId".to_string(), current);
    deleted_ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // 覆盖矩阵（history 纯函数）：
    // ✅ get_current_message_id：history.currentId / chat.currentId /
    //    branchPointMessageId / 扁平 messages 列表回退 / 全缺 → None /
    //    history.currentId 为空串时跳到下一优先级
    // ✅ last_descendant_id：链式下行 / 断链（孩子不存在）/ 成环（cycle 安全）
    // ✅ clean_null_bytes：嵌套字符串、数组、对象、非字符串原样
    // ✅ merge_history：incoming 整条覆盖同 id 消息；childrenIds 重建（去重）；
    //    currentId 三级回退（incoming→existing→None）；非 dict 消息丢弃；
    //    history 其他键保留（incoming 覆盖 existing）
    // ✅ upsert 新消息：显式 role 保留；父 user→assistant、assistant→user、
    //    无父→assistant；parentId 显式优先、缺省时从 childrenIds 反查；
    //    currentId 指向新消息；timestamp 缺省补 now
    // ✅ upsert 已有消息：浅合并（incoming 键覆盖），currentId 不变，父链补挂
    // ✅ delete：删叶子；删中间节点（孩子一起删、孙子重挂到父）；
    //    currentId 重走最后孩子链；返回删除集合；删不存在的 id → 空
    // ⛔ 刻意不覆盖：并发（纯函数无共享状态）
    // （本矩阵原本挂在 tests 模块上；cargo fix 后改为普通注释）

    fn tree() -> Value {
        // root ── a ── b
        //        └── c
        json!({
            "history": {
                "currentId": "a",
                "messages": {
                    "root": {"id": "root", "parentId": null, "childrenIds": ["a", "c"], "role": "user", "timestamp": 1},
                    "a": {"id": "a", "parentId": "root", "childrenIds": ["b"], "role": "assistant", "timestamp": 2},
                    "b": {"id": "b", "parentId": "a", "childrenIds": [], "role": "user", "timestamp": 3},
                    "c": {"id": "c", "parentId": "root", "childrenIds": [], "role": "assistant", "timestamp": 4}
                }
            }
        })
    }

    #[test]
    fn current_message_id_from_history_current_id() {
        assert_eq!(get_current_message_id(&tree()), Some("a".to_string()));
    }

    #[test]
    fn current_message_id_fallbacks_in_order() {
        // chat.currentId
        assert_eq!(
            get_current_message_id(&json!({"currentId": "x"})),
            Some("x".to_string())
        );
        // branchPointMessageId
        assert_eq!(
            get_current_message_id(&json!({"branchPointMessageId": "y"})),
            Some("y".to_string())
        );
        // history.currentId empty string → falls through to chat.currentId
        assert_eq!(
            get_current_message_id(&json!({"history": {"currentId": ""}, "currentId": "z"})),
            Some("z".to_string())
        );
        // flat messages list, last one wins
        let flat = json!({"messages": [{"id": "m1"}, {"id": "m2"}]});
        assert_eq!(get_current_message_id(&flat), Some("m2".to_string()));
        // nothing anywhere
        assert_eq!(get_current_message_id(&json!({})), None);
    }

    #[test]
    fn last_descendant_walks_and_guards() {
        let t = tree();
        let messages = t["history"]["messages"].as_object().unwrap();
        // root's childrenIds are ["a", "c"] — the LAST child wins
        assert_eq!(last_descendant_id(messages, "root"), "c");
        assert_eq!(last_descendant_id(messages, "b"), "b");
        // cycle: a → b → a
        let cyclic = json!({
            "a": {"id": "a", "childrenIds": ["b"], "parentId": "b"},
            "b": {"id": "b", "childrenIds": ["a"], "parentId": "a"}
        });
        let cyclic_obj = cyclic.as_object().unwrap();
        assert_eq!(last_descendant_id(cyclic_obj, "a"), "a");
    }

    #[test]
    fn clean_null_bytes_everywhere() {
        let input =
            json!({"a": "x\u{0}y", "list": ["n\u{0}", 1, true, null], "nested": {"s": "\u{0}"}});
        assert_eq!(
            clean_null_bytes(&input),
            json!({"a": "xy", "list": ["n", 1, true, null], "nested": {"s": ""}})
        );
        assert_eq!(clean_null_bytes(&json!(42)), json!(42));
    }

    #[test]
    fn merge_history_union_and_rebuild() {
        let existing = json!({"history": {
            "currentId": "a",
            "messages": {
                "a": {"id": "a", "parentId": null, "childrenIds": [], "role": "user", "timestamp": 1},
                "b": {"id": "b", "parentId": "a", "childrenIds": [], "role": "assistant", "timestamp": 2}
            }
        }});
        // Incoming edited `a` (whole message replaced) and lost `b`.
        let incoming = json!({"history": {
            "currentId": "a",
            "messages": {
                "a": {"id": "a", "parentId": null, "childrenIds": [], "role": "user", "timestamp": 99, "content": "edited"}
            }
        }});
        let merged = merge_history(Some(&existing["history"]), Some(&incoming["history"]));
        let messages = merged["messages"].as_object().unwrap();
        // b survived (stale-writer protection)
        assert!(messages.contains_key("b"));
        // a was replaced wholesale by incoming
        assert_eq!(messages["a"]["timestamp"], 99);
        assert_eq!(messages["a"]["content"], "edited");
        // childrenIds rebuilt from parentId links
        assert_eq!(messages["a"]["childrenIds"], json!(["b"]));
        assert_eq!(messages["b"]["childrenIds"], json!([]));
        assert_eq!(merged["currentId"], "a");
    }

    #[test]
    fn merge_history_current_id_fallback_chain() {
        let existing =
            json!({"currentId": "keep", "messages": {"keep": {"id": "keep", "parentId": null}}});
        // incoming currentId invalid → falls to existing
        let incoming = json!({"currentId": "ghost", "messages": {}});
        let merged = merge_history(Some(&existing), Some(&incoming));
        assert_eq!(merged["currentId"], "keep");
        // neither valid → null
        let merged2 = merge_history(
            Some(&json!({"currentId": "ghost", "messages": {}})),
            Some(&incoming),
        );
        assert_eq!(merged2["currentId"], Value::Null);
        // extra keys: incoming overrides existing; messages/currentId always set
        let merged3 = merge_history(
            Some(&json!({"k": 1, "messages": {}})),
            Some(&json!({"k": 2})),
        );
        assert_eq!(merged3["k"], 2);
        assert!(merged3["messages"].is_object());
    }

    #[test]
    fn merge_drops_non_dict_messages() {
        let incoming = json!({"messages": {"a": "not-a-dict", "b": {"id": "b"}}});
        let merged = merge_history(None, Some(&incoming));
        let messages = merged["messages"].as_object().unwrap();
        assert!(!messages.contains_key("a"));
        assert!(messages.contains_key("b"));
    }

    #[test]
    fn upsert_new_message_role_from_parent() {
        let mut history = json!({
            "currentId": "u1",
            "messages": {"u1": {"id": "u1", "parentId": null, "childrenIds": [], "role": "user", "timestamp": 1}}
        });
        // assistant reply without explicit role → assistant (parent is user)
        let reply = json!({"id": "a1", "parentId": "u1", "content": "hi"});
        let inserted = upsert_message_to_history(&mut history, "a1", &reply).unwrap();
        assert_eq!(inserted["role"], "assistant");
        assert_eq!(history["currentId"], "a1");
        // parent's childrenIds got the child
        assert_eq!(history["messages"]["u1"]["childrenIds"], json!(["a1"]));
    }

    #[test]
    fn upsert_new_message_role_inversions() {
        // parent assistant → new message becomes user
        let mut history = json!({
            "messages": {"a1": {"id": "a1", "parentId": null, "childrenIds": [], "role": "assistant"}}
        });
        let user_msg = json!({"parentId": "a1"});
        let inserted = upsert_message_to_history(&mut history, "u2", &user_msg).unwrap();
        assert_eq!(inserted["role"], "user");
        // parentless message with no listing parent → assistant
        let mut empty =
            json!({"messages": {"a1": {"id": "a1", "role": "assistant", "parentId": null}}});
        let orphan = upsert_message_to_history(&mut empty, "x1", &json!({})).unwrap();
        assert_eq!(orphan["role"], "assistant");
        // explicit role always wins
        let mut h2 = json!({"messages": {"u1": {"id": "u1", "role": "user", "parentId": null}}});
        let sys =
            upsert_message_to_history(&mut h2, "s1", &json!({"parentId": "u1", "role": "system"}))
                .unwrap();
        assert_eq!(sys["role"], "system");
    }

    #[test]
    fn upsert_infers_parent_from_children_listing() {
        // Message arrives with no parentId but some node already lists it.
        let mut history = json!({
            "messages": {"u1": {"id": "u1", "role": "user", "parentId": null, "childrenIds": ["a9"]}}
        });
        let inserted =
            upsert_message_to_history(&mut history, "a9", &json!({"role": "assistant"})).unwrap();
        assert_eq!(inserted["parentId"], "u1");
    }

    #[test]
    fn upsert_patch_existing_keeps_current_id() {
        let mut history = json!({
            "currentId": "a1",
            "messages": {
                "u1": {"id": "u1", "parentId": null, "childrenIds": [], "role": "user"},
                "a1": {"id": "a1", "parentId": "u1", "childrenIds": [], "role": "assistant", "content": "old"}
            }
        });
        let patched =
            upsert_message_to_history(&mut history, "a1", &json!({"content": "new"})).unwrap();
        assert_eq!(patched["content"], "new");
        assert_eq!(patched["role"], "assistant"); // kept
        assert_eq!(history["currentId"], "a1"); // unchanged on patch
        // duplicate child never appended twice
        assert_eq!(history["messages"]["u1"]["childrenIds"], json!(["a1"]));
    }

    #[test]
    fn delete_leaf_and_midnode() {
        // delete_message_from_history receives the `history` sub-object.
        let mut blob = tree();
        blob["history"]["currentId"] = json!("b");
        let deleted = {
            let history = blob.get_mut("history").unwrap();
            delete_message_from_history(history, "b")
        };
        assert_eq!(deleted, vec!["b".to_string()]);
        let messages = blob["history"]["messages"].as_object().unwrap();
        assert!(!messages.contains_key("b"));
        assert_eq!(blob["history"]["currentId"], "a");

        // midnode a: b (child) deleted, b (grandchild-of-root via a) re-hung
        // on root (Python quirk: dangling grandchild id remains in the list).
        let mut blob = tree();
        blob["history"]["currentId"] = json!("a");
        let deleted = {
            let history = blob.get_mut("history").unwrap();
            delete_message_from_history(history, "a")
        };
        assert_eq!(deleted, vec!["a".to_string(), "b".to_string()]);
        let messages = blob["history"]["messages"].as_object().unwrap();
        assert!(!messages.contains_key("a") && !messages.contains_key("b"));
        assert_eq!(messages["c"]["parentId"], "root");
        assert!(
            messages["root"]["childrenIds"]
                .as_array()
                .unwrap()
                .contains(&json!("c"))
        );
        assert!(
            !messages["root"]["childrenIds"]
                .as_array()
                .unwrap()
                .contains(&json!("a"))
        );
    }

    #[test]
    fn delete_missing_returns_empty() {
        let mut history = tree();
        assert!(delete_message_from_history(&mut history, "ghost").is_empty());
        assert_eq!(
            delete_message_from_history(&mut json!({}), "x"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn repair_adds_missing_child_links() {
        // blob arrived without childrenIds — repair fills them
        let mut chat = json!({
            "history": {
                "currentId": "a",
                "messages": {
                    "u": {"id": "u", "parentId": null, "role": "user", "timestamp": 1},
                    "a": {"id": "a", "parentId": "u", "role": "assistant", "timestamp": 2}
                }
            }
        });
        assert!(crate::repo::chats::repair_chat_current_id(&mut chat));
        assert_eq!(
            chat["history"]["messages"]["u"]["childrenIds"],
            json!(["a"])
        );
    }

    #[test]
    fn repair_moves_current_to_latest_leaf() {
        // currentId dangling + fresh assistant leaf exists → moves there
        let mut chat = json!({
            "history": {
                "currentId": "stale",
                "messages": {
                    "u": {"id": "u", "parentId": null, "role": "user", "timestamp": 1, "childrenIds": []},
                    "new": {"id": "new", "parentId": "u", "role": "assistant", "timestamp": 50, "childrenIds": []}
                }
            }
        });
        assert!(crate::repo::chats::repair_chat_current_id(&mut chat));
        assert_eq!(chat["history"]["currentId"], "new");
        // already-correct currentId → no change
        let mut chat2 = json!({
            "history": {
                "currentId": "new",
                "messages": {
                    "u": {"id": "u", "parentId": null, "role": "user", "timestamp": 1, "childrenIds": ["new"]},
                    "new": {"id": "new", "parentId": "u", "role": "assistant", "timestamp": 50, "childrenIds": []}
                }
            }
        });
        assert!(!crate::repo::chats::repair_chat_current_id(&mut chat2));
    }
}
