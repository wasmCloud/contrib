mod bindings;
mod config;
mod provider;

use provider::MysqlProvider;

/// Capability providers are native executables, so the entrypoint is the same as any other Rust
/// binary, `main()`. Typically the `main` function is kept simple and the provider logic is
/// implemented in a separate module. Head to the `provider.rs` file to see the implementation of
/// the `BlankSlateProvider`.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    MysqlProvider::run().await?;
    eprintln!("MySQL provider exiting");
    Ok(())
}
