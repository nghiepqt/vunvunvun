fn main() {
    println!("cargo:rerun-if-changed=app.manifest");
    println!("cargo:rerun-if-changed=../../packaging/msix/Assets/AppIcon.ico");

    #[cfg(target_os = "windows")]
    {
        if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
            let mut res = winresource::WindowsResource::new();
            res.set_manifest_file("app.manifest");
            res.set_icon("../../packaging/msix/Assets/AppIcon.ico");
            if let Err(err) = res.compile() {
                panic!("failed to compile Windows resources: {err}");
            }
        }
    }
}
