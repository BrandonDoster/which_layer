// Embeds assets/icon.ico as icon resource 1: Explorer/taskbar use it, and the
// tray loads it back (tray.rs). Regenerate the icon with tools/icon.py.
fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new().set_icon("assets/icon.ico").compile().expect("embedding icon");
    }
}
