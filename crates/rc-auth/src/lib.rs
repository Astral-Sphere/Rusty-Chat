//! # rc-auth
//!
//! Authentication primitives, byte-compatible with open-webui 0.11.3:
//! - JWT HS256 with claims `{id, exp, iat, jti}` (cookie name `token`)
//! - bcrypt (cost 12, 72-byte semantics) / argon2 (auto-detected by prefix)
//! - `sk-` API keys (32 hex chars, stored plaintext)
//!
//! Hashing stays out of the data layer: repositories accept a verifier.

pub mod api_key;
pub mod duration;
pub mod jwt;
pub mod password;

pub use api_key::generate_api_key;
pub use duration::{ParsedDuration, parse_duration};
pub use jwt::{Claims, create_token, decode_token};
pub use password::{
    HashAlgorithm, PASSWORD_BCRYPT_MAX_BYTES, hash_password, hash_password_bcrypt,
    placeholder_hash, validate_password, verify_password,
};
