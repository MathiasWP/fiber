fn main() {
    // The app icon is compiled into the binary — on macOS, `tauri dev` sets the
    // Dock icon from it at runtime rather than from a bundle. tauri-build only
    // watches the config file, so regenerating the icons on their own left the
    // old one embedded and the Dock unchanged until something else forced a
    // rebuild.
    println!("cargo:rerun-if-changed=icons");

    tauri_build::build()
}
