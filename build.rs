//! Embeds the application icon and version information into the Windows
//! executables, so Explorer, Task Manager and shortcuts show them.

fn main() {
    println!("cargo:rerun-if-changed=assets/icons/app.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/icons/app.ico")
        .set("ProductName", "Obsid Ambilight")
        .set("FileDescription", "Obsid Ambilight");
    res.compile().expect("compile Windows resources");
}
