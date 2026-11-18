//! Windows: icon, manifest (asInvoker, PerMonitorV2) and version info.

fn main() {
    println!("cargo:rerun-if-changed=assets/guardian.ico");
    println!("cargo:rerun-if-changed=assets/guardian-setup.manifest");
    // The target, not the host: cross builds must embed the resources too.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        windows_resources();
    }
}

#[cfg(windows)]
fn windows_resources() {
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/guardian.ico")
        .set_manifest_file("assets/guardian-setup.manifest")
        .set("ProductName", "Guardian Setup")
        .set("FileDescription", "Guardian 3.0 agent setup");
    if let Err(e) = res.compile() {
        println!("cargo:warning=cannot embed Windows resources: {e}");
    }
}

#[cfg(not(windows))]
fn windows_resources() {
    // winresource is only a build dependency on Windows hosts; a cross build
    // from another OS ships without the icon (CI builds on Windows).
    println!("cargo:warning=Windows resources are embedded only when building on Windows");
}
