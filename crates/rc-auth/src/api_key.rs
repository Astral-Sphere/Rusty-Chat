//! API key format — open-webui: `sk-` + `str(uuid.uuid4()).replace('-','')`
//! (32 lowercase hex chars). Keys are stored plaintext (unique index) and
//! presented via the `Authorization: Bearer sk-…` or `x-api-key` headers.

/// Generates a new key in the exact open-webui format.
pub fn generate_api_key() -> String {
    let id = uuid::Uuid::new_v4();
    format!("sk-{}", id.simple())
}

/// True when the string has the `sk-` prefix shape (used to route Bearer
/// tokens to API-key authentication instead of JWT).
pub fn is_api_key(token: &str) -> bool {
    token.starts_with("sk-")
}

#[cfg(test)]
mod tests {
    use super::*;

    // 覆盖矩阵：
    // ✅ 格式：sk- + 32 小写 hex；与 uuid4 simple 形式一致
    // ✅ 唯一性：两次生成不同
    // ✅ is_api_key：sk- 前缀判定；JWT 不误判
    // ⛔ 刻意不覆盖：存储唯一性（由 DB 唯一索引保证，repo 测试覆盖）

    #[test]
    fn format_matches_open_webui() {
        let key = generate_api_key();
        assert!(key.starts_with("sk-"));
        let hex = &key[3..];
        assert_eq!(hex.len(), 32);
        assert!(
            hex.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
    }

    #[test]
    fn keys_are_unique() {
        assert_ne!(generate_api_key(), generate_api_key());
    }

    #[test]
    fn is_api_key_classification() {
        assert!(is_api_key(&generate_api_key()));
        assert!(!is_api_key("eyJhbGciOiJIUzI1NiJ9.x.y"));
        assert!(!is_api_key("sk"));
        assert!(is_api_key("sk-"));
    }
}
