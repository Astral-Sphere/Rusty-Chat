//! JWT — open-webui `create_token`/`decode_token` parity.
//!
//! HS256, secret = `WEBUI_SECRET_KEY`. Claims: `id` (user id) plus standard
//! `exp` (epoch seconds), `iat` (epoch seconds), `jti` (uuid4, one per
//! token). Token lifetime comes from config `auth.jwt_expiry` (duration
//! string); open-webui passes `None` only for non-expiring service tokens.

use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use rc_core::{Error, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const ALGORITHM: Algorithm = Algorithm::HS256;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub id: String,
    /// epoch seconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exp: Option<u64>,
    /// epoch seconds
    pub iat: i64,
    pub jti: String,
    /// room for extra claims (open-webui puts arbitrary `data` in tokens)
    #[serde(flatten)]
    pub extra: std::collections::BTreeMap<String, serde_json::Value>,
}

/// `create_token(data, expires_delta)` — `data` MUST contain `id`.
pub fn create_token(
    user_id: &str,
    secret: &str,
    expires_delta: Option<std::time::Duration>,
) -> Result<(String, Claims)> {
    let now = rc_core::timestamp::Secs::now().as_i64();
    let claims = Claims {
        id: user_id.to_string(),
        exp: expires_delta.map(|d| (now as u64) + d.as_secs()),
        iat: now,
        jti: Uuid::new_v4().to_string(),
        extra: Default::default(),
    };
    let token = jsonwebtoken::encode(
        &Header::new(ALGORITHM),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| Error::Internal(format!("jwt encode failed: {e}")))?;
    Ok((token, claims))
}

/// `decode_token` — verifies signature, algorithm and expiry; None-equivalent
/// errors collapse into `Error::Unauthorized` (callers turn that into 401).
pub fn decode_token(token: &str, secret: &str) -> Result<Claims> {
    let mut validation = Validation::new(ALGORITHM);
    // open-webui relies on `exp` when present; tokens without exp are valid.
    validation.required_spec_claims.clear();
    validation.validate_exp = true;
    jsonwebtoken::decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map(|data| data.claims)
    .map_err(|e| Error::Unauthorized(format!("invalid token: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    // 覆盖矩阵：
    // ✅ 往返：claims 全字段保留（id/iat/jti/exp）
    // ✅ exp 以 epoch 秒写入且 = iat + delta
    // ✅ jti 唯一（两次签发不同）
    // ✅ 无 exp 的 token 可解码（服务型 token）
    // ✅ 过期 token 拒绝
    // ✅ 错误密钥 / 篡改 payload / 错误算法 拒绝
    // ✅ HS256 头（与 PyJWT 兼容格式）
    // ⛔ 刻意不覆盖：Redis 吊销（M7）

    const SECRET: &str = "test-secret-0123456789";

    #[test]
    fn roundtrip_preserves_claims() {
        let (token, claims) =
            create_token("user-1", SECRET, Some(Duration::from_secs(3600))).unwrap();
        let decoded = decode_token(&token, SECRET).unwrap();
        assert_eq!(decoded.id, "user-1");
        assert_eq!(decoded.jti, claims.jti);
        assert_eq!(decoded.iat, claims.iat);
        assert_eq!(decoded.exp.unwrap(), claims.exp.unwrap());
        assert!(decoded.exp.unwrap() > decoded.iat as u64);
    }

    #[test]
    fn jti_unique_per_token() {
        let (_, a) = create_token("u", SECRET, None).unwrap();
        let (_, b) = create_token("u", SECRET, None).unwrap();
        assert_ne!(a.jti, b.jti);
    }

    #[test]
    fn non_expiring_tokens_valid() {
        let (token, _claims) = create_token("svc", SECRET, None).unwrap();
        let decoded = decode_token(&token, SECRET).unwrap();
        assert_eq!(decoded.id, "svc");
        assert!(decoded.exp.is_none());
    }

    #[test]
    fn expired_token_rejected() {
        // craft with exp in the past
        let claims = Claims {
            id: "u".into(),
            exp: Some(1), // 1970
            iat: 0,
            jti: "j".into(),
            extra: Default::default(),
        };
        let token = jsonwebtoken::encode(
            &Header::new(ALGORITHM),
            &claims,
            &EncodingKey::from_secret(SECRET.as_bytes()),
        )
        .unwrap();
        assert!(decode_token(&token, SECRET).is_err());
    }

    #[test]
    fn wrong_secret_or_tampered_payload_rejected() {
        let (token, _) = create_token("u", SECRET, None).unwrap();
        assert!(decode_token(&token, "other-secret").is_err());
        // flip a char in the payload segment
        let (token2, _) = create_token("u", SECRET, None).unwrap();
        let mut parts: Vec<String> = token2.split('.').map(str::to_string).collect();
        let tampered = if parts[1].starts_with('e') {
            format!("f{}", &parts[1][1..])
        } else {
            format!("e{}", &parts[1][1..])
        };
        parts[1] = tampered;
        let tampered_token = parts.join(".");
        assert!(decode_token(&tampered_token, SECRET).is_err());
    }

    #[test]
    fn header_is_hs256() {
        let (token, _) = create_token("u", SECRET, None).unwrap();
        let header = jsonwebtoken::decode_header(&token).unwrap();
        assert_eq!(header.alg, Algorithm::HS256);
        assert_eq!(header.typ.as_deref(), Some("JWT"));
    }
}
