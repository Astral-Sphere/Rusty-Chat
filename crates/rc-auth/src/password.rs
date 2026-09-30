//! Password hashing — open-webui `utils/auth.py` parity.
//!
//! open-webui 0.11.3 defaults to bcrypt (gensalt default cost 12); argon2 is
//! selected by `PASSWORD_HASH_ALGORITHM=argon2`. `verify_password` sniffs the
//! `$argon2` prefix so stored hashes verify regardless of the current
//! algorithm. bcrypt only consumes the first 72 bytes of the password;
//! open-webui's verify TRUNCATES silently while signup REJECTS longer
//! passwords (`validate_password`) — reproduce both behaviors exactly.

use argon2::PasswordVerifier;
use argon2::password_hash::CustomizedPasswordHasher;
use rc_core::{Error, Result};

pub const PASSWORD_BCRYPT_MAX_BYTES: usize = 72;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HashAlgorithm {
    #[default]
    Bcrypt,
    Argon2,
}

impl HashAlgorithm {
    pub fn from_env_str(s: Option<&str>) -> Result<Self> {
        match s.unwrap_or("bcrypt") {
            "bcrypt" => Ok(Self::Bcrypt),
            "argon2" => Ok(Self::Argon2),
            other => Err(Error::Internal(format!(
                "Unsupported PASSWORD_HASH_ALGORITHM: {other}"
            ))),
        }
    }
}

/// `get_password_hash` — hash with the configured algorithm.
pub fn hash_password(password: &str, algorithm: HashAlgorithm) -> Result<String> {
    match algorithm {
        HashAlgorithm::Bcrypt => hash_password_bcrypt(password),
        HashAlgorithm::Argon2 => {
            // argon2 0.6 (password-hash 0.6): raw salt bytes + phc-string output.
            let salt: [u8; 16] = rand::random();
            let hasher = argon2::Argon2::default();
            hasher
                .hash_password_customized(
                    password.as_bytes(),
                    &salt,
                    None,
                    None,
                    argon2::Params::default(),
                )
                .map(|phc| phc.to_string())
                .map_err(|e| Error::Internal(format!("argon2 hash failed: {e}")))
        }
    }
}

/// bcrypt path (`bcrypt.gensalt()` default cost = 12).
pub fn hash_password_bcrypt(password: &str) -> Result<String> {
    let hashed = bcrypt::hash(password, bcrypt::DEFAULT_COST)
        .map_err(|e| Error::Internal(format!("bcrypt hash failed: {e}")))?;
    Ok(hashed)
}

/// `verify_password` — argon2 detected by `$argon2` prefix; otherwise bcrypt
/// with the first 72 BYTES of the UTF-8 password (silent truncation).
pub fn verify_password(plain: &str, hashed: &str) -> bool {
    if hashed.is_empty() {
        return false;
    }
    if hashed.starts_with("$argon2") {
        return argon2::Argon2::default()
            .verify_password(plain.as_bytes(), hashed)
            .is_ok();
    }
    // str::len() is BYTE length — identical semantics to as_bytes().len().
    let bytes = &plain.as_bytes()[..plain.len().min(PASSWORD_BCRYPT_MAX_BYTES)];
    bcrypt::verify(bytes, hashed).unwrap_or(false)
}

/// `validate_password` — signup path: bcrypt rejects passwords longer than
/// 72 bytes up front (open-webui PASSWORD_TOO_LONG). Regex policy is not
/// enabled by default in open-webui 0.11.3 (`ENABLE_PASSWORD_VALIDATION`).
pub fn validate_password(password: &str, algorithm: HashAlgorithm) -> Result<()> {
    if algorithm == HashAlgorithm::Bcrypt && password.len() > PASSWORD_BCRYPT_MAX_BYTES {
        return Err(Error::BadRequest(
            "The password is too long. It must be no longer than 72 bytes.".to_string(),
        ));
    }
    Ok(())
}

/// open-webui computes a bcrypt hash of the literal string `placeholder` at
/// import time; unknown-user sign-ins burn a verify against it so response
/// timing cannot reveal account existence (CWE-208).
pub fn placeholder_hash() -> String {
    // Unwrap is sound: a literal cannot exceed bcrypt's input limits.
    hash_password_bcrypt("placeholder").expect("static placeholder hash")
}

#[cfg(test)]
mod tests {
    use super::*;

    // 覆盖矩阵：
    // ✅ bcrypt 往返、错误密码拒绝
    // ✅ 72 字节：verify 截断语义（73 字节密码前 72 字节相同 → 验证通过）
    // ✅ validate_password 对 >72 字节（含多字节字符边界）拒绝
    // ✅ hash 与 verify 的 72 字节截断一致（对齐 Python bcrypt；signup 由
    //    validate_password 把关）
    // ✅ argon2 往返 + $argon2 前缀自动识别 + 坏 hash 拒绝
    // ✅ 空 hash 拒绝
    // ✅ HashAlgorithm::from_env_str（None→bcrypt、argon2、未知→Err）
    // ⛔ 刻意不覆盖：正则密码策略（OWU 默认关闭）

    #[test]
    fn bcrypt_roundtrip() {
        let h = hash_password_bcrypt("secret").unwrap();
        assert!(h.starts_with("$2"));
        assert!(verify_password("secret", &h));
        assert!(!verify_password("wrong", &h));
    }

    #[test]
    fn bcrypt_truncates_at_72_bytes_on_verify() {
        // Same first 72 ASCII bytes; the 73rd differs → both verify.
        let base = "a".repeat(72);
        let h = hash_password_bcrypt(&base).unwrap();
        assert!(verify_password(&base, &h));
        assert!(verify_password(&format!("{base}EXTRA"), &h));
        // But a different prefix fails
        assert!(!verify_password(&"b".repeat(72), &h));
    }

    #[test]
    fn validate_password_rejects_over_72_bytes_bcrypt() {
        validate_password(&"a".repeat(72), HashAlgorithm::Bcrypt).unwrap();
        assert!(validate_password(&"a".repeat(73), HashAlgorithm::Bcrypt).is_err());
        // 36 four-byte emoji = 144 bytes → rejected even though 36 chars
        assert!(validate_password(&"🔥".repeat(36), HashAlgorithm::Bcrypt).is_err());
        // argon2 has no 72-byte limit
        validate_password(&"a".repeat(200), HashAlgorithm::Argon2).unwrap();
    }

    #[test]
    fn argon2_roundtrip_and_prefix_detection() {
        let h = hash_password("secret", HashAlgorithm::Argon2).unwrap();
        assert!(h.starts_with("$argon2"));
        assert!(verify_password("secret", &h));
        assert!(!verify_password("wrong", &h));
        // malformed argon2 hash → false, never panic
        assert!(!verify_password("x", "$argon2id$v=19$m=65536,t=3,p=4$"));
    }

    #[test]
    fn empty_and_invalid_hashes() {
        assert!(!verify_password("x", ""));
        assert!(!verify_password("x", "not-a-hash"));
    }

    #[test]
    fn placeholder_is_bcrypt_of_literal() {
        let ph = placeholder_hash();
        assert!(verify_password("placeholder", &ph));
        assert!(!verify_password("anything-else", &ph));
    }

    #[test]
    fn hash_algorithm_from_env_str() {
        // None defaults to bcrypt (open-webui 0.11.3 default)
        assert_eq!(
            HashAlgorithm::from_env_str(None).unwrap(),
            HashAlgorithm::Bcrypt
        );
        assert_eq!(
            HashAlgorithm::from_env_str(Some("bcrypt")).unwrap(),
            HashAlgorithm::Bcrypt
        );
        assert_eq!(
            HashAlgorithm::from_env_str(Some("argon2")).unwrap(),
            HashAlgorithm::Argon2
        );
        assert!(HashAlgorithm::from_env_str(Some("scrypt")).is_err());
        assert!(HashAlgorithm::from_env_str(Some("")).is_err());
    }

    #[test]
    fn bcrypt_hash_truncates_at_72_bytes_like_python() {
        // Python bcrypt.hashpw silently truncates at 72 bytes and the Rust
        // crate matches — a 73-byte input hashes to its 72-byte prefix.
        // Signup is gated by validate_password; this pins library parity.
        let h = hash_password_bcrypt(&"a".repeat(73)).unwrap();
        assert!(verify_password(&"a".repeat(72), &h));
        assert!(verify_password(&"a".repeat(73), &h));
        assert!(!verify_password(&"b".repeat(72), &h));
        assert!(hash_password("x", HashAlgorithm::Bcrypt).is_ok());
    }
}
