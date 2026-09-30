//! Repository integration tests — SQLite (always) + Postgres (gated by
//! `RC_TEST_PG_URL`). Schema comes from our own bootstrap DDL (ground truth:
//! open-webui 0.11.3 alembic head).
//!
//! 覆盖矩阵：
//! ✅ users：insert 默认值/非法头像回退、按 id/email(大小写不敏感)/api_key 查询、
//!   patch 更新 + updated_at 前移、计数、删除（api_key 级联）
//! ✅ auths：signup 建 auth+user、重复 email 拒绝、authenticate 正确/错误密码/
//!   未知邮箱（仍烧 placeholder 哈希）/inactive 账号、改密、改邮箱同步 user、
//!   api key 生命周期（create/get/touch/delete）
//! ✅ chats：insert 默认标题/dual-write chat_message、get+读时修复、update
//!   顶层合并+history 合并（旧写者不丢消息）、标题/标签更新、pin/archive 切换、
//!   列表过滤（internal 排除/文件夹/pinned/archived）、搜索、分享（share→
//!   快照→refresh→unshare→delete 清行）、消息 upsert（blob+行）、消息删除、
//!   删除聊天（行+快照清理）、last_read_at
//! ✅ config 引擎：seed 只插缺失、upsert 更新+插入、get_or、all
//! ✅ 双方言：SQLite 必跑；RC_TEST_PG_URL 门控 PG（同断言同语义）
//! ⛔ 刻意不覆盖：并发写竞争（由单连接测试模型 + PG for update 语义另行覆盖，M5）

use rc_db::bootstrap::{Dialect, bootstrap};
use rc_db::repo::auths::{self, SignupParams, VerifyPassword};
use rc_db::repo::chats::{self, ChatTitleId, NewChatParams};
use rc_db::repo::config::ConfigEngine;
use rc_db::repo::{chat_messages, shared_chats, tags, users};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, Database, DatabaseConnection, EntityTrait};
use serde_json::json;

/// Acquires a single AnyConnection via a pool (sqlx 0.9 style).
async fn any_conn(url: &str) -> (sqlx::AnyPool, sqlx::pool::PoolConnection<sqlx::Any>) {
    rc_db::install_drivers();
    let pool = sqlx::AnyPool::connect(url).await.unwrap();
    let conn = pool.acquire().await.unwrap();
    (pool, conn)
}

struct NoVerify;
impl VerifyPassword for NoVerify {
    async fn verify(&self, _hash: &str) -> bool {
        false
    }
}

struct YesVerify;
impl VerifyPassword for YesVerify {
    async fn verify(&self, _hash: &str) -> bool {
        true
    }
}

/// Bootstraps a fresh SQLite database and returns a sea-orm connection.
async fn sqlite_db() -> (DatabaseConnection, tempfile::TempDir) {
    rc_db::install_drivers();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let url = format!("sqlite://{}?mode=rwc", path.display());
    let (_pool, mut conn) = any_conn(&url).await;
    bootstrap(&mut conn, Dialect::Sqlite).await.unwrap();
    drop(conn);
    let db = Database::connect(&url).await.unwrap();
    (db, dir)
}

/// PG variant: creates (and later drops) a scratch database.
async fn pg_db() -> Option<(DatabaseConnection, String)> {
    let Ok(url) = std::env::var("RC_TEST_PG_URL") else {
        return None;
    };
    rc_db::install_drivers();
    let trimmed = url.trim_end_matches('/');
    let cut = trimmed.rfind('/').filter(|i| !trimmed[..*i].ends_with(':'));
    let base = match cut {
        Some(i) => &trimmed[..i],
        None => trimmed,
    };
    let (_pool, mut conn) = any_conn(&format!("{base}/postgres")).await;
    sqlx::raw_sql("DROP DATABASE IF EXISTS rc_repo_test WITH (FORCE);")
        .execute(&mut *conn)
        .await
        .ok();
    sqlx::raw_sql("CREATE DATABASE rc_repo_test;")
        .execute(&mut *conn)
        .await
        .unwrap();
    drop(conn);
    let db_url = format!("{base}/rc_repo_test");
    let (_pool2, mut conn) = any_conn(&db_url).await;
    bootstrap(&mut conn, Dialect::Postgres).await.unwrap();
    drop(conn);
    let db = Database::connect(&db_url).await.unwrap();
    Some((db, db_url))
}

/// Get-or-create a user with a per-id unique email (functional unique index
/// on lower(email) is live in this schema).
async fn seed_user(db: &DatabaseConnection, id: &str) -> users::User {
    if let Some(existing) = users::get_user_by_id(db, id).await.unwrap() {
        return existing;
    }
    let email = format!("{id}@example.com");
    users::insert_new_user(
        db,
        users::NewUserParams {
            id,
            name: "Tester",
            email: &email,
            profile_image_url: None,
            role: Some("user"),
            username: None,
            oauth: None,
        },
    )
    .await
    .unwrap()
    .unwrap()
}

// ---------- users ----------

async fn users_crud(db: &DatabaseConnection) {
    // insert defaults
    let u = seed_user(db, "u1").await;
    assert_eq!(u.role.as_deref(), Some("user"));
    assert_eq!(u.profile_image_url.as_deref(), Some("/user.png"));
    let now = rc_core::timestamp::Secs::now().as_i64();
    assert!(u.created_at.unwrap() >= now - 5, "created_at in seconds");

    // invalid profile image falls back
    let u2 = users::insert_new_user(
        db,
        users::NewUserParams {
            id: "u2",
            name: "T2",
            email: "T2@Example.COM",
            profile_image_url: Some("javascript:alert(1)"),
            role: Some("admin"),
            username: Some("t2"),
            oauth: None,
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(u2.profile_image_url.as_deref(), Some("/user.png"));

    // case-insensitive email lookup
    let found = users::get_user_by_email(db, "t2@example.com")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.id, "u2");

    // count
    assert_eq!(users::get_num_users(db).await.unwrap(), 2);

    // patch updates + updated_at bump
    let before = users::get_user_by_id(db, "u1").await.unwrap().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let after = users::update_user_by_id(
        db,
        "u1",
        users::UserPatch {
            name: Some("Renamed".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(after.name, "Renamed");
    assert!(after.updated_at.unwrap() > before.updated_at.unwrap());

    // missing user → None
    assert!(
        users::update_user_by_id(db, "ghost", users::UserPatch::default())
            .await
            .unwrap()
            .is_none()
    );

    // api_key lifecycle + lookup
    auths::api_keys::create(db, "u1", "sk-abc123")
        .await
        .unwrap();
    let by_key = users::get_user_by_api_key(db, "sk-abc123")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(by_key.id, "u1");
    assert!(
        users::get_user_by_api_key(db, "sk-none")
            .await
            .unwrap()
            .is_none()
    );
    auths::api_keys::touch_last_used(db, "sk-abc123")
        .await
        .unwrap();
    assert!(
        auths::api_keys::get_by_user_id(db, "u1")
            .await
            .unwrap()
            .is_some()
    );
    assert!(auths::api_keys::delete_by_user_id(db, "u1").await.unwrap());
    assert!(
        users::get_user_by_api_key(db, "sk-abc123")
            .await
            .unwrap()
            .is_none()
    );

    // delete user
    assert!(users::delete_user_by_id(db, "u2").await.unwrap());
    assert!(users::get_user_by_id(db, "u2").await.unwrap().is_none());
}

// ---------- auths ----------

async fn auths_flow(db: &DatabaseConnection) {
    // signup
    let created = auths::insert_new_auth(
        db,
        SignupParams {
            email: "Ada@Example.com",
            password_hash: "$2b$12$hash",
            name: "Ada",
            profile_image_url: None,
            role: Some("user"),
            oauth: None,
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(created.role.as_deref(), Some("user"));
    assert_eq!(created.email.as_deref(), Some("Ada@Example.com"));

    // duplicate email rejected
    let dup = auths::insert_new_auth(
        db,
        SignupParams {
            email: "ada@example.com",
            password_hash: "x",
            name: "Dup",
            profile_image_url: None,
            role: None,
            oauth: None,
        },
    )
    .await;
    assert!(dup.is_err(), "duplicate email must fail");

    // authenticate: correct credentials (verifier=true)
    let ok = auths::authenticate_user(db, "ada@example.com", "pw", &YesVerify, "$fake")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ok.id, created.id);

    // wrong password
    assert!(
        auths::authenticate_user(db, "ada@example.com", "pw", &NoVerify, "$fake")
            .await
            .unwrap()
            .is_none()
    );

    // unknown email: placeholder hash still verified (timing parity), then None
    assert!(
        auths::authenticate_user(db, "ghost@x.com", "pw", &YesVerify, "$fake")
            .await
            .unwrap()
            .is_none()
    );

    // deactivate → authenticate refuses even with right password
    let conn = db;
    let row = rc_db::entity::auth::Entity::find_by_id(&created.id)
        .one(conn)
        .await
        .unwrap()
        .unwrap();
    let mut am: rc_db::entity::auth::ActiveModel = row.into();
    am.active = Set(Some(false));
    am.update(conn).await.unwrap();
    assert!(
        auths::authenticate_user(db, "ada@example.com", "pw", &YesVerify, "$fake")
            .await
            .unwrap()
            .is_none()
    );

    // password update reactivates nothing but works after re-activation
    assert!(
        auths::update_user_password_by_id(db, &created.id, "$new")
            .await
            .unwrap()
    );
    assert!(
        !auths::update_user_password_by_id(db, "ghost", "$x")
            .await
            .unwrap()
    );

    // email update mirrors to user row
    assert!(
        auths::update_email_by_id(db, &created.id, "new@example.com")
            .await
            .unwrap()
    );
    let mirrored = users::get_user_by_email(db, "new@example.com")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(mirrored.id, created.id);

    // delete auth+user pair
    assert!(auths::delete_auth_by_id(db, &created.id).await.unwrap());
    assert!(
        users::get_user_by_id(db, &created.id)
            .await
            .unwrap()
            .is_none()
    );
}

// ---------- chats ----------

fn blob(title: &str) -> serde_json::Value {
    json!({
        "title": title,
        "models": ["m1"],
        "history": {
            "currentId": "m1",
            "messages": {
                "m0": {"id": "m0", "parentId": null, "childrenIds": ["m1"], "role": "user", "content": "hello", "timestamp": 1},
                "m1": {"id": "m1", "parentId": "m0", "childrenIds": [], "role": "assistant", "content": "world", "timestamp": 2}
            }
        }
    })
}

async fn chats_flow(db: &DatabaseConnection) {
    seed_user(db, "owner").await;

    // insert + dual-write
    let chat = chats::insert_new_chat(
        db,
        NewChatParams {
            id: "c1",
            user_id: "owner",
            chat: &blob("My Chat"),
            folder_id: None,
            variables: None,
            internal_meta: None,
            timer_at: None,
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(chat.title.as_deref(), Some("My Chat"));
    assert_eq!(chat.current_message_id.as_deref(), Some("m1"));
    assert_eq!(chat.archived, Some(false));
    assert_eq!(chat.meta.as_ref().unwrap(), &json!({}));
    // timestamps in seconds, last_read_at set
    let now = rc_core::timestamp::Secs::now().as_i64();
    assert!(chat.created_at.unwrap() >= now - 5);
    assert!(chat.last_read_at.is_some());

    // dual-write: two rows keyed by composite id
    let msgs = chat_messages::get_messages_by_chat_id(db, "c1")
        .await
        .unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0].id, "c1-m0");
    assert_eq!(msgs[0].role.as_deref(), Some("user"));
    assert_eq!(msgs[1].id, "c1-m1");
    // legacy flat message id has no explicit role → skipped, but m0/m1 do

    // default title
    let mut no_title = blob("x");
    no_title.as_object_mut().unwrap().remove("title");
    let c2 = chats::insert_new_chat(
        db,
        NewChatParams {
            id: "c2",
            user_id: "owner",
            chat: &no_title,
            folder_id: None,
            variables: None,
            internal_meta: None,
            timer_at: None,
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(c2.title.as_deref(), Some("New Chat"));

    // null bytes scrubbed on insert
    let dirty = json!({"title": "bad\u{0}title", "history": {"messages": {}}});
    let c3 = chats::insert_new_chat(
        db,
        NewChatParams {
            id: "c3",
            user_id: "owner",
            chat: &dirty,
            folder_id: None,
            variables: None,
            internal_meta: None,
            timer_at: None,
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(c3.title.as_deref(), Some("badtitle"));

    // get by id + by id&user; wrong user → None; missing → None
    assert!(chats::get_chat_by_id(db, "c1").await.unwrap().is_some());
    assert!(
        chats::get_chat_by_id_and_user_id(db, "c1", "owner")
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        chats::get_chat_by_id_and_user_id(db, "c1", "intruder")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        chats::get_chat_by_id_and_user_id(db, "ghost", "owner")
            .await
            .unwrap()
            .is_none()
    );

    // update: top-level merge, history MERGE keeps messages the stale writer lost
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let before = chats::get_chat_by_id(db, "c1").await.unwrap().unwrap();
    let stale = json!({
        "title": "Renamed",
        "history": {"currentId": "m0", "messages": {
            "m0": {"id": "m0", "parentId": null, "childrenIds": [], "role": "user", "content": "hello", "timestamp": 1}
        }}
    });
    let updated = chats::update_chat_by_id(db, "c1", &stale, true)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.title.as_deref(), Some("Renamed"));
    // m1 survived the stale write
    let messages = updated.chat.as_ref().unwrap()["history"]["messages"]
        .as_object()
        .unwrap();
    assert!(messages.contains_key("m1"));
    // childrenIds rebuilt
    assert_eq!(messages["m0"]["childrenIds"], json!(["m1"]));
    assert!(updated.updated_at.unwrap() > before.updated_at.unwrap());

    // touch=false keeps updated_at
    let untouched = chats::update_chat_by_id(db, "c1", &json!({"summary": "s"}), false)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(untouched.updated_at, updated.updated_at);

    // title-only update: column AND blob stay in sync, so a later
    // update_chat_by_id re-derives the NEW title, not a stale one
    // (regression: column-only write let branch switches clobber generated
    // titles back to the blob's "New Chat")
    let titled = chats::update_chat_title_by_id(db, "c1", "Final")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(titled.title.as_deref(), Some("Final"));
    assert_eq!(
        titled.chat.as_ref().unwrap()["title"],
        json!("Final"),
        "blob title must be updated together with the column"
    );
    let rederived = chats::update_chat_by_id(db, "c1", &json!({"summary": "x"}), false)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        rederived.title.as_deref(),
        Some("Final"),
        "title must survive a later partial blob update"
    );

    // message upsert via blob path (new assistant message m2)
    let _ = chats::upsert_message_to_chat_by_id_and_message_id(
        db, "c1", "m2",
        &json!({"id": "m2", "parentId": "m1", "role": "assistant", "content": "again", "timestamp": 3}),
    ).await.unwrap().unwrap();
    let chat_after = chats::get_chat_by_id(db, "c1").await.unwrap().unwrap();
    assert_eq!(chat_after.current_message_id.as_deref(), Some("m2"));
    let row = chat_messages::get_message_by_id(db, "c1-m2")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.content, Some(json!("again")));

    // message delete (leaf m2) — blob + row cleaned
    let (_, deleted_ids) = chats::delete_message_from_chat_by_id_and_message_id(db, "c1", "m2")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(deleted_ids, vec!["m2".to_string()]);
    assert!(
        chat_messages::get_message_by_id(db, "c1-m2")
            .await
            .unwrap()
            .is_none()
    );

    // pin/archive toggles (touch last_read_at too)
    let pinned = chats::toggle_chat_pinned_by_id(db, "c1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pinned.pinned, Some(true));
    let unpinned = chats::toggle_chat_pinned_by_id(db, "c1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unpinned.pinned, Some(false));

    // folder move updates the COLUMN (blob-merge path never touches it, OWU parity)
    let infolder = chats::update_chat_folder_by_id(db, "c1", Some("f1"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(infolder.folder_id.as_deref(), Some("f1"));

    // archive clears folder
    let archived = chats::toggle_chat_archive_by_id(db, "c1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(archived.archived, Some(true));
    assert_eq!(archived.folder_id, None);
    let unarchived = chats::toggle_chat_archive_by_id(db, "c1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unarchived.archived, Some(false));

    // archive-all / unarchive-all
    assert!(
        chats::archive_all_chats_by_user_id(db, "owner")
            .await
            .unwrap()
    );
    assert_eq!(
        chats::get_archived_chat_list_by_user_id(db, "owner")
            .await
            .unwrap()
            .len(),
        3
    );
    assert!(
        chats::unarchive_all_chats_by_user_id(db, "owner")
            .await
            .unwrap()
    );
    assert!(
        chats::get_archived_chat_list_by_user_id(db, "owner")
            .await
            .unwrap()
            .is_empty()
    );
}

async fn chat_lists_flow(db: &DatabaseConnection) {
    seed_user(db, "owner").await;
    for (id, title) in [
        ("l1", "alpha"),
        ("l2", "Beta"),
        ("l3", "gamma"),
        ("l4", "delta"),
    ] {
        chats::insert_new_chat(
            db,
            NewChatParams {
                id,
                user_id: "owner",
                chat: &blob(title),
                folder_id: None,
                variables: None,
                internal_meta: None,
                timer_at: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
    }
    // Column paths (blob merge never touches these columns, OWU parity):
    // l2 pinned, l3 foldered, l4 archived (archive clears folder_id), internal.
    chats::toggle_chat_pinned_by_id(db, "l2")
        .await
        .unwrap()
        .unwrap();
    chats::update_chat_folder_by_id(db, "l3", Some("folderA"))
        .await
        .unwrap()
        .unwrap();
    chats::insert_new_chat(
        db,
        NewChatParams {
            id: "internal",
            user_id: "owner",
            chat: &json!({"title": "internal", "history": {"messages": {}}}),
            folder_id: None,
            variables: None,
            internal_meta: Some(&json!({"internal": true, "type": "note"})),
            timer_at: None,
        },
    )
    .await
    .unwrap()
    .unwrap();
    chats::toggle_chat_archive_by_id(db, "l4")
        .await
        .unwrap()
        .unwrap();

    let ids =
        |list: &Vec<ChatTitleId>| -> Vec<String> { list.iter().map(|c| c.id.clone()).collect() };

    // default sidebar list: excludes pinned, foldered, archived, internal
    let list =
        chats::get_chat_title_id_list_by_user_id(db, "owner", false, false, false, None, None)
            .await
            .unwrap();
    assert_eq!(ids(&list), vec!["l1"], "only l1 qualifies by default");

    // include pinned + folders
    let list = chats::get_chat_title_id_list_by_user_id(db, "owner", false, true, true, None, None)
        .await
        .unwrap();
    assert_eq!(ids(&list), vec!["l1", "l2", "l3"]);

    // include archived
    let list = chats::get_chat_title_id_list_by_user_id(db, "owner", true, true, true, None, None)
        .await
        .unwrap();
    assert_eq!(ids(&list), vec!["l1", "l2", "l3", "l4"]);

    // order: updated_at desc, id asc tiebreak
    let ts: Vec<i64> = list.iter().map(|c| c.updated_at).collect();
    let mut sorted = ts.clone();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    assert_eq!(ts, sorted, "updated_at desc");
    for pair in list.windows(2) {
        if pair[0].updated_at == pair[1].updated_at {
            assert!(pair[0].id <= pair[1].id, "id tiebreak asc");
        }
    }

    // pagination
    let page =
        chats::get_chat_title_id_list_by_user_id(db, "owner", true, true, true, Some(1), Some(1))
            .await
            .unwrap();
    assert_eq!(page.len(), 1);

    // search (case-insensitive substring)
    let hits = chats::get_chat_list_by_search(db, "owner", "ALPH")
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "l1");

    // folder listing
    let foldered = chats::get_chats_by_folder_id(db, "folderA", 0, 50)
        .await
        .unwrap();
    assert_eq!(foldered.len(), 1);
    assert_eq!(foldered[0].id, "l3");
}

async fn chat_share_tags_flow(db: &DatabaseConnection) {
    seed_user(db, "owner").await;
    chats::insert_new_chat(
        db,
        NewChatParams {
            id: "s1",
            user_id: "owner",
            chat: &blob("Shared Chat"),
            folder_id: None,
            variables: None,
            internal_meta: None,
            timer_at: None,
        },
    )
    .await
    .unwrap()
    .unwrap();

    // share → snapshot exists, share_id set
    let shared = chats::share_chat(db, "s1").await.unwrap().unwrap();
    let share_id = shared.share_id.clone().unwrap();
    // idempotent: second share refreshes the same snapshot id
    let reshare = chats::share_chat(db, "s1").await.unwrap().unwrap();
    assert_eq!(reshare.share_id.as_deref(), Some(share_id.as_str()));

    // public view via share token
    let public = chats::get_chat_by_share_id(db, &share_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(public.id, share_id);
    assert_eq!(public.title.as_deref(), Some("Shared Chat"));
    let (snapshot, original) = shared_chats::get_shared_chats_by_user(db, "owner")
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(snapshot.id, share_id);
    assert_eq!(original.as_ref().unwrap().id, "s1");

    // refresh picks up new content
    chats::update_chat_title_by_id(db, "s1", "Renamed Shared")
        .await
        .unwrap();
    chats::share_chat(db, "s1").await.unwrap();
    let public = chats::get_chat_by_share_id(db, &share_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(public.title.as_deref(), Some("Renamed Shared"));

    // unshare: snapshot row gone, chat keeps share_id (OWU parity)
    assert!(
        chats::delete_shared_chat_by_chat_id(db, "s1")
            .await
            .unwrap()
    );
    assert!(
        chats::get_chat_by_share_id(db, &share_id)
            .await
            .unwrap()
            .is_none()
    );
    let after_unshare = chats::get_chat_by_id(db, "s1").await.unwrap().unwrap();
    assert!(after_unshare.share_id.is_some());

    // tags: replace flow creates rows, orphans cleaned
    chats::update_chat_tags_by_id(db, "s1", &["Work Stuff", "ideas"], "owner")
        .await
        .unwrap();
    let mut tag_ids: Vec<String> = tags::get_tags_by_user_id(db, "owner")
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.id)
        .collect();
    tag_ids.sort();
    assert_eq!(tag_ids, vec!["ideas", "work_stuff"]);
    let s1 = chats::get_chat_by_id(db, "s1").await.unwrap().unwrap();
    assert_eq!(
        s1.meta.as_ref().unwrap()["tags"],
        json!(["work_stuff", "ideas"])
    );
    // "none" tag id is filtered out
    chats::update_chat_tags_by_id(db, "s1", &["None"], "owner")
        .await
        .unwrap();
    let s1 = chats::get_chat_by_id(db, "s1").await.unwrap().unwrap();
    assert_eq!(s1.meta.as_ref().unwrap()["tags"], json!([]));
    // orphan cleanup: old tags removed since no chat references them
    assert!(
        tags::get_tags_by_user_id(db, "owner")
            .await
            .unwrap()
            .is_empty()
    );

    // delete chat: messages + snapshot cleaned (share_id was recreated above)
    chats::share_chat(db, "s1").await.unwrap();
    assert!(
        chats::delete_chat_by_id_and_user_id(db, "s1", "owner")
            .await
            .unwrap()
    );
    assert!(chats::get_chat_by_id(db, "s1").await.unwrap().is_none());
    assert!(
        chat_messages::get_messages_by_chat_id(db, "s1")
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        shared_chats::get_shared_chats_by_user(db, "owner")
            .await
            .unwrap()
            .is_empty()
    );
    // wrong owner cannot delete
    chats::insert_new_chat(
        db,
        NewChatParams {
            id: "s2",
            user_id: "owner",
            chat: &blob("x"),
            folder_id: None,
            variables: None,
            internal_meta: None,
            timer_at: None,
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        !chats::delete_chat_by_id_and_user_id(db, "s2", "intruder")
            .await
            .unwrap()
    );
    assert!(chats::get_chat_by_id(db, "s2").await.unwrap().is_some());

    // delete-all
    assert!(chats::delete_chats_by_user_id(db, "owner").await.unwrap());
    let survivor = users::get_user_by_id(db, "owner").await.unwrap();
    assert!(survivor.is_some(), "user survives");
    assert!(chats::get_chat_by_id(db, "s2").await.unwrap().is_none());
}

async fn chat_last_read_flow(db: &DatabaseConnection) {
    seed_user(db, "owner").await;
    chats::insert_new_chat(
        db,
        NewChatParams {
            id: "r1",
            user_id: "owner",
            chat: &blob("Read test"),
            folder_id: None,
            variables: None,
            internal_meta: None,
            timer_at: None,
        },
    )
    .await
    .unwrap()
    .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    chats::update_chat_by_id(db, "r1", &json!({"summary": "changed"}), true)
        .await
        .unwrap();

    // unread: updated_at > last_read_at
    let (ts, was_unread) = chats::update_chat_last_read_at_by_id(db, "r1", "owner")
        .await
        .unwrap()
        .unwrap();
    assert!(was_unread);
    assert!(ts >= rc_core::timestamp::Secs::now().as_i64() - 5);
    // second read: not unread
    let (_, was_unread) = chats::update_chat_last_read_at_by_id(db, "r1", "owner")
        .await
        .unwrap()
        .unwrap();
    assert!(!was_unread);
    // wrong user → None
    assert!(
        chats::update_chat_last_read_at_by_id(db, "r1", "intruder")
            .await
            .unwrap()
            .is_none()
    );
}

// ---------- config ----------

async fn config_engine_flow(db: &DatabaseConnection) {
    let engine = ConfigEngine::new(db.clone());
    use std::collections::BTreeMap;
    let mut defaults = BTreeMap::new();
    defaults.insert("ui.enable_signup".to_string(), json!(true));
    defaults.insert("ui.default_user_role".to_string(), json!("pending"));

    assert_eq!(engine.seed_defaults(&defaults).await.unwrap(), 2);
    // reseed inserts nothing
    assert_eq!(engine.seed_defaults(&defaults).await.unwrap(), 0);

    // get / get_or
    assert_eq!(
        engine.get("ui.enable_signup").await.unwrap(),
        Some(json!(true))
    );
    assert_eq!(
        engine.get_or("missing.key", json!(42)).await.unwrap(),
        json!(42)
    );

    // upsert inserts and updates
    engine
        .upsert("ui.enable_signup", &json!(false))
        .await
        .unwrap();
    assert_eq!(
        engine.get("ui.enable_signup").await.unwrap(),
        Some(json!(false))
    );
    engine
        .upsert("brand.new", &json!({"a": [1, 2]}))
        .await
        .unwrap();
    assert_eq!(
        engine.get("brand.new").await.unwrap(),
        Some(json!({"a": [1, 2]}))
    );

    let all = engine.all().await.unwrap();
    assert!(all.len() >= 3);
    assert!(engine.delete("brand.new").await.unwrap());
    assert!(engine.get("brand.new").await.unwrap().is_none());
}

/// Runs one flow against a FRESH SQLite database and a FRESH Postgres
/// database (flows are not isolated against each other's leftovers).
// The scratch PG database is shared between test threads; this file-scope
// lock must cover CREATE DATABASE too (a static inside the macro would be
// duplicated per test).
static PG_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

macro_rules! everywhere {
    ($flow:ident) => {{
        let (db, _dir) = sqlite_db().await;
        $flow(&db).await;
        if std::env::var("RC_TEST_PG_URL").is_ok() {
            let _guard = PG_LOCK.lock().await;
            if let Some((db, url)) = pg_db().await {
                $flow(&db).await;
                let (_pool, mut conn) = any_conn(&url).await;
                sqlx::raw_sql("DROP DATABASE IF EXISTS rc_repo_test WITH (FORCE);")
                    .execute(&mut *conn)
                    .await
                    .ok();
            }
        }
    }};
}

#[tokio::test]
async fn users_crud_both_dialects() {
    everywhere!(users_crud);
}

#[tokio::test]
async fn auths_flow_both_dialects() {
    everywhere!(auths_flow);
}

#[tokio::test]
async fn chats_flow_both_dialects() {
    everywhere!(chats_flow);
}

#[tokio::test]
async fn chat_lists_flow_both_dialects() {
    everywhere!(chat_lists_flow);
}

#[tokio::test]
async fn chat_share_tags_flow_both_dialects() {
    everywhere!(chat_share_tags_flow);
}

#[tokio::test]
async fn chat_last_read_flow_both_dialects() {
    everywhere!(chat_last_read_flow);
}

#[tokio::test]
async fn config_engine_flow_both_dialects() {
    everywhere!(config_engine_flow);
}
