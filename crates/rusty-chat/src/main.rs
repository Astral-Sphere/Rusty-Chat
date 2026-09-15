use anyhow::Result;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> Result<()> {
    // M0: binary placeholder. The `serve` command (axum + embedded frontend)
    // lands in M1; subcommands arrive with their features:
    //   rusty-chat serve            — M1
    //   rusty-chat migrate-check    — M0 (this bootstrap module)
    //   rusty-chat create-admin     — M1
    //   rusty-chat reindex          — M2
    println!("rusty-chat {VERSION} (M0 skeleton)");
    Ok(())
}
