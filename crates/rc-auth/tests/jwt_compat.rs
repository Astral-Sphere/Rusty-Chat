//! Cross-implementation JWT compatibility contract against PyJWT (the
//! library open-webui itself uses). The reference token below was generated
//! with:
//!
//! ```python
//! import jwt
//! jwt.encode(
//!     {"id": "compat-user", "exp": 4102444800, "iat": 1757890000,
//!      "jti": "fixed-jti-123"},
//!     "compat-test-secret", algorithm="HS256")
//! ```
//!
//! If Rusty-Chat decodes this token, our HS256 wire format, claim names and
//! epoch-second timestamps match open-webui's tokens exactly (and vice
//! versa: `rc_auth::create_token` output is structurally identical).
//!
//! 覆盖矩阵（真实基准互操作，非记忆编造——基准 token/hash 见上方生成脚本）:
//! ✅ PyJWT 签发的 HS256 token 可被 decode_token（claim 名/单位/结构一致）
//! ✅ 错误密钥的 PyJWT token 拒绝
//! ✅ Python bcrypt 4.x hash 可被 verify_password（cost 12 基准）
//! ⛔ 刻意不覆盖：argon2 的 Python 互操作（OWU argon2 走 passlib 同为
//!    PHC 字符串，单元测试已锁 $argon2 前缀语义）；JWT 算法语义在
//!    src/jwt.rs 单测覆盖

use rc_auth::{Claims, decode_token};

const PYJWT_TOKEN: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJpZCI6ImNvbXBhdC11c2VyIiwiZXhwIjo0MTAyNDQ0ODAwLCJpYXQiOjE3NTc4OTAwMDAsImp0aSI6ImZpeGVkLWp0aS0xMjMifQ.gybJPPYzNCz3ilpPdXFuxd0rusA81-cLMYbwulSyLHs";
const SECRET: &str = "compat-test-secret";

#[test]
fn decodes_pyjwt_token() {
    let claims: Claims = decode_token(PYJWT_TOKEN, SECRET).unwrap();
    assert_eq!(claims.id, "compat-user");
    assert_eq!(claims.exp, Some(4_102_444_800));
    assert_eq!(claims.iat, 1_757_890_000);
    assert_eq!(claims.jti, "fixed-jti-123");
    assert!(claims.extra.is_empty());
}

#[test]
fn rejects_pyjwt_token_with_wrong_secret() {
    assert!(decode_token(PYJWT_TOKEN, "other").is_err());
}

// bcrypt cross-check: hash produced by Python bcrypt 4.x via the open-webui
// fixture venv (bcrypt.hashpw(b"secret", bcrypt.gensalt(rounds=12))).
// Verifies our verifier accepts real open-webui password hashes.
#[test]
fn verifies_python_bcrypt_hash() {
    let py_hash = "$2b$12$.bcqxNDLsR4.pPY55.knjupuI0DPUmkeJ56rvrmmHgP3Gte1Vh2Mu";
    assert!(rc_auth::verify_password("secret", py_hash));
    assert!(!rc_auth::verify_password("Secret", py_hash));
}
