// Prevents additional console window on Windows in release, DO NOT REMOVE!!
// Only the GUI build wants this — the headless MCP server is a console program.
#![cfg_attr(all(not(debug_assertions), feature = "gui"), windows_subsystem = "windows")]

/// The same binary is both the app and its MCP server: `fiber mcp` speaks MCP
/// over stdio against the same collections on disk, with no window and no
/// running app. Argument parsing happens before Tauri starts, so the MCP path
/// never initialises a GUI.
///
/// The headless build (`--no-default-features`) drops the GUI entirely and only
/// provides `fiber mcp`, which is what ships in a container.
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

    #[cfg(feature = "gui")]
    fiber_lib::run();

    #[cfg(not(feature = "gui"))]
    {
        eprintln!("This is the headless build — the only command is `fiber mcp`.");
        std::process::exit(2);
    }
}
