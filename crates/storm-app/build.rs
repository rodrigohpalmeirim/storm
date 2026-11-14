//! Embeds the app icon (and version info) into `storm.exe`.
//!
//! gpui loads icon resource #1 of the running exe for the window and taskbar,
//! and Explorer/Start use the same resource, so this is the only icon wiring.

fn main() {
    let icon = "../../assets/icon.ico";
    println!("cargo:rerun-if-changed={icon}");
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon_with_id(icon, "1").set("FileDescription", "Storm").set("ProductName", "Storm");
        if let Err(e) = res.compile() {
            panic!("couldn't embed the Windows resources: {e}");
        }
    }
}
