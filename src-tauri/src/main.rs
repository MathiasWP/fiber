// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// The same binary is both the app and its MCP server: `fiber mcp` speaks MCP
/// over stdio against the same collections on disk, with no window and no
/// running app. Argument parsing happens before Tauri starts, so the MCP path
/// never initialises a GUI.
fn main() {
    if std::env::args().nth(1).as_deref() == Some("mcp") {
        let runtime = match tokio::runtime::Runtime::new() {
            Ok(runtime) => runtime,
            Err(err) => {
                eprintln!("could not start: {err}");
                std::process::exit(1);
            }
        };
        if let Err(err) = runtime.block_on(fiber_lib::mcp::serve()) {
            // stdout is the protocol channel, so diagnostics go to stderr.
            eprintln!("mcp server stopped: {err}");
            std::process::exit(1);
        }
        return;
    }

    fiber_lib::run();
}
