fn main() {
    tauri_build::build();

    // Embed Info.plist into the binary so TCC (microphone permission prompt)
    // works during `tauri dev`, where the app runs outside a .app bundle.
    #[cfg(target_os = "macos")]
    {
        println!("cargo:rerun-if-changed=Info.plist");
        println!("cargo:rustc-link-arg=-Wl,-sectcreate,__TEXT,__info_plist,Info.plist");
    }
}
