pub mod windows_api;

use zbus::interface;

pub struct Daemon;

// Interface name must match `wgaf_common::INTERFACE_NAME` (zbus requires a
// string literal here, so it can't reference the constant directly).
#[interface(name = "org.wgaf.Daemon1")]
impl Daemon {
    async fn ping(&self) -> String {
        "pong".to_string()
    }

    async fn version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }
}
