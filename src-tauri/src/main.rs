// Prevents additional console window on Windows in release, DO NOT REMOVE!!
// Only the GUI build wants this — the headless MCP server is a console program.
#![cfg_attr(
    all(not(debug_assertions), feature = "gui"),
    windows_subsystem = "windows"
)]

/// The same binary is both the app and its MCP server: `fiber mcp` speaks MCP
/// over stdio against the same collections on disk, with no window and no
/// running app. Argument parsing happens before Tauri starts, so the MCP path
/// never initialises a GUI.
///
/// The headless build (`--no-default-features`) drops the GUI entirely and only
/// provides `fiber mcp`, which is what ships in a container.
fn main() {
    if std::env::args().nth(1).as_deref() == Some("mcp") {
        // `export-secrets` is setup rather than serving: it reads the keychain,
        // prints the map a containerised copy needs, and exits. No runtime, no
        // protocol. See mcp::export_secrets for why it is allowed to do the one
        // thing nothing else in the app does.
        if std::env::args().nth(2).as_deref() == Some("export-secrets") {
            // `--to <path>` writes the sealed file a container reads live
            // instead of printing the map. Same credentials, but the app can
            // then keep it current as you sign in, rather than the container
            // holding whatever was true when it started.
            let target = match std::env::args().nth(3).as_deref() {
                Some("--to") => match std::env::args().nth(4) {
                    Some(path) => Some(std::path::PathBuf::from(path)),
                    None => {
                        eprintln!("--to needs a path.");
                        std::process::exit(1);
                    }
                },
                Some(other) => {
                    eprintln!("Unknown option {other}. The only one is --to <path>.");
                    std::process::exit(1);
                }
                None => None,
            };
            let result = match &target {
                Some(path) => fiber_lib::mcp::export_secrets_to(path),
                None => fiber_lib::mcp::export_secrets(),
            };
            if let Err(err) = result {
                eprintln!("{err}");
                std::process::exit(1);
            }
            return;
        }

        // The key that seals that file. It stays out of the mounted directory:
        // the app keeps it in the keychain, the container gets it injected.
        if std::env::args().nth(2).as_deref() == Some("file-key") {
            if let Err(err) = fiber_lib::mcp::print_file_key() {
                eprintln!("{err}");
                std::process::exit(1);
            }
            return;
        }

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
