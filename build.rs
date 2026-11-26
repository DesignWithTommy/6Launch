// The exe's icon (assets/i95.ico).
fn main() {
    println!("cargo:rerun-if-changed=assets/i95.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut r = winresource::WindowsResource::new();
        r.set_icon("assets/i95.ico");
        r.set("ProductName", "i95");
        r.set("FileDescription", "i95");
        r.compile().expect("icon resource");
    }
}
