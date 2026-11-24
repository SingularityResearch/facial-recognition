//! On Windows the program's file carries the app's icon (Explorer, the taskbar, the window) and says what it is.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let icon = "../../assets/icons/icon.ico";
    println!("cargo:rerun-if-changed={icon}");
    let mut file = winresource::WindowsResource::new();
    file.set_icon(icon); // (as resource 1: the window asks for it by that number)
    file.set("ProductName", "Face Recognizer");
    file.set("FileDescription", "Face Recognizer");
    file.compile().expect("the Windows SDK's resource compiler makes the program's icon");
}
