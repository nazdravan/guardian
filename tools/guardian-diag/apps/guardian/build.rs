//! Windows only: icon, VERSIONINFO and the asInvoker manifest. A missing resource compiler is a
//! warning (cross-compiles still link) unless GUARDIAN_REQUIRE_WINRES=1 (CI release builds).
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=guardian.manifest");
    println!("cargo:rerun-if-changed=../../../whitelist-gui/guardian.ico");
    println!("cargo:rerun-if-env-changed=GUARDIAN_REQUIRE_WINRES");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../../whitelist-gui/guardian.ico")
        .set_manifest_file("guardian.manifest")
        .set("ProductName", "Guardian")
        .set("FileDescription", "Guardian — agent diagnostics and settings")
        .set("OriginalFilename", "Guardian.exe")
        .set("InternalName", "Guardian");
    if let Err(e) = res.compile() {
        if std::env::var_os("GUARDIAN_REQUIRE_WINRES").is_some() {
            println!("cargo::error=embedding Windows resources failed: {e}");
            return;
        }
        println!("cargo:warning=Guardian.exe built without icon/manifest: {e}");
    }
}
