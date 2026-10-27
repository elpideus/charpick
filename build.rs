fn main() {
    println!("cargo:rerun-if-changed=charpick.rc");
    println!("cargo:rerun-if-changed=charpick.manifest");
    println!("cargo:rerun-if-changed=assets");
    // The icon, version info and manifest are Windows resources.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let result = embed_resource::compile("charpick.rc", embed_resource::NONE);
    if cfg!(windows) {
        result.manifest_required().unwrap();
    } else if let Err(e) = result.manifest_optional() {
        // Cross-checking from another OS without a resource compiler.
        println!("cargo:warning=Windows resources not embedded: {e}");
    }
}
