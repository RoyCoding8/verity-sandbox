fn main() {
    let windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    let attributes = if windows {
        tauri_build::Attributes::new()
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest())
    } else {
        tauri_build::Attributes::new()
    };
    tauri_build::try_build(attributes).expect("failed to build Tauri resources");
    if windows {
        println!("cargo:rerun-if-changed=windows.manifest");
        let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
        let manifest = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
            .join("windows.manifest");
        let resource = out.join("common-controls.rc");
        std::fs::write(
            &resource,
            format!("1 24 {:?}", manifest.to_string_lossy().replace('\\', "/")),
        )
        .unwrap();
        embed_resource::compile_for_everything(resource, embed_resource::NONE)
            .manifest_required()
            .unwrap();
    }
}
