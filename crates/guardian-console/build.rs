//! Windows: icon, manifest (asInvoker, PerMonitorV2) and version info.

fn main() {
    println!("cargo:rerun-if-changed=assets/guardian.ico");
    println!("cargo:rerun-if-changed=assets/guardian-console.manifest");
    #[cfg(windows)]
    windows_resources();
}

#[cfg(windows)]
fn windows_resources() {
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/guardian.ico")
        .set_manifest_file("assets/guardian-console.manifest")
        .set("ProductName", "Guardian Console")
        .set("FileDescription", "Guardian 3.0 teacher console");
    if let Err(e) = res.compile() {
        println!("cargo:warning=cannot embed Windows resources: {e}");
    }
}
