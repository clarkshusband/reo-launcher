fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("icons/icon.ico");
        println!("cargo:rerun-if-changed=icons/icon.ico");

        let version = env!("CARGO_PKG_VERSION");
        res.set("CompanyName", "drako");
        res.set("ProductName", "Reo Launcher");
        res.set("FileDescription", "Reo Launcher");
        res.set(
            "LegalCopyright",
            "© 2026 drako. ReoLauncher, © Reo Inc. and Contributors.",
        );
        res.set("OriginalFilename", "reo_app.exe");
        res.set("InternalName", "reo_app");
        res.set("FileVersion", version);
        res.set("ProductVersion", version);

        res.compile()
            .expect("failed to embed Windows icon resource");
    }
}
