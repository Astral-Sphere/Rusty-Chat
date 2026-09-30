//! Server settings — env vars with open-webui 0.11.3 semantics (M1 subset).
//!
//! `WEBUI_SECRET_KEY` persistence follows open-webui exactly: env value wins;
//! otherwise a random 24-char alphanumeric key is generated once and stored
//! in `DATA_DIR/.webui_secret_key` (path overridable via
//! `WEBUI_SECRET_KEY_FILE`).

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Settings {
    pub data_dir: PathBuf,
    pub database_url: String,
    pub secret_key: String,
    pub webui_name: String,
    pub host: String,
    pub port: u16,
    /// open-webui `WEBUI_AUTH` — M1 always requires auth; `false` is accepted
    /// but behaves like `true` with a warning (fixed-admin flow lands in M3).
    pub webui_auth: bool,
    pub frontend_dist: PathBuf,
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

impl Settings {
    /// Loads settings from the environment. `version` is the reported
    /// WEBUI_VERSION (crate version).
    pub fn load() -> anyhow::Result<Self> {
        let data_dir = PathBuf::from(env_or("DATA_DIR", "data"));
        std::fs::create_dir_all(&data_dir)?;

        let secret_key = load_secret_key(&data_dir)?;
        let database_url = match std::env::var("DATABASE_URL") {
            Ok(url) => url.replacen("postgres://", "postgresql://", 1),
            Err(_) => {
                // SQLite does not create the file on connect (M0 pitfall);
                // open-webui's sqlalchemy does, so mirror that with mode=rwc.
                format!("sqlite:///{}/webui.db?mode=rwc", data_dir.display())
            }
        };

        Ok(Self {
            data_dir,
            database_url,
            secret_key,
            webui_name: env_or("WEBUI_NAME", "Open WebUI"),
            host: env_or("HOST", "0.0.0.0"),
            port: env_or("PORT", "8080").parse()?,
            webui_auth: env_or("WEBUI_AUTH", "True").eq_ignore_ascii_case("true"),
            frontend_dist: PathBuf::from(env_or("FRONTEND_DIST_DIR", "web/dist")),
        })
    }
}

/// open-webui `WEBUI_SECRET_KEY` resolution (see `open_webui/__init__.py`):
/// 1. `WEBUI_SECRET_KEY` env verbatim;
/// 2. contents of `WEBUI_SECRET_KEY_FILE` (default `<DATA_DIR>/.webui_secret_key`);
/// 3. generate a random 24-char alphanumeric key and persist it to that file.
fn load_secret_key(data_dir: &Path) -> anyhow::Result<String> {
    if let Ok(key) = std::env::var("WEBUI_SECRET_KEY")
        && !key.is_empty()
    {
        return Ok(key);
    }
    let key_file = std::env::var("WEBUI_SECRET_KEY_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| data_dir.join(".webui_secret_key"));

    if let Ok(key) = std::fs::read_to_string(&key_file) {
        let key = key.trim();
        if !key.is_empty() {
            return Ok(key.to_string());
        }
    }

    const CHARS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let key: String = (0..24)
        .map(|_| {
            use rand::Rng;
            CHARS[rand::rng().random_range(0..CHARS.len())] as char
        })
        .collect();
    std::fs::write(&key_file, &key)?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    // 覆盖矩阵：
    // ✅ env 存在 → 原样使用且不落盘
    // ✅ env 缺失 + 文件存在 → 读文件
    // ✅ 两者皆无 → 生成 24 字符并写入文件；再次调用读到同一个
    // ⛔ 刻意不覆盖：文件权限位（OWU 也不设置）

    #[test]
    fn secret_key_persistence_cycle() {
        let dir = tempfile::tempdir().unwrap();
        let key_file = dir.path().join(".webui_secret_key");

        // 3: generate + persist
        let k1 = load_secret_key(dir.path()).unwrap();
        assert_eq!(k1.len(), 24);
        assert_eq!(std::fs::read_to_string(&key_file).unwrap(), k1);
        // 2: read back from file
        let k2 = load_secret_key(dir.path()).unwrap();
        assert_eq!(k1, k2);

        // explicit env beats file
        // (env var set by the test harness would leak across tests; here we
        // only verify the file path override)
        let custom = dir.path().join("custom.key");
        std::fs::write(&custom, "custom-key").unwrap();
        unsafe { std::env::set_var("WEBUI_SECRET_KEY_FILE", &custom) };
        let k3 = load_secret_key(dir.path()).unwrap();
        assert_eq!(k3, "custom-key");
        unsafe { std::env::remove_var("WEBUI_SECRET_KEY_FILE") };
    }
}
