// Start node server.js and show the web UI in a WebView window.
// This shell uses no sidecar plugin and no shell plugin. A plain
// std::process::Command is sufficient.
//
// A GUI shell has no console. Without this, Windows attaches one and it stays
// on screen behind the window. Debug builds keep the console for the logs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command};
use std::time::{Duration, Instant};
#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// CREATE_NO_WINDOW. A console subsystem child (node) opens its own console
/// window when the parent has none. This flag suppresses it.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn wait_for_port(port: u16, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

fn main() {
    // server.js is at the repository root, one directory above this crate.
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("desktop/ has a parent directory");

    let mut command = Command::new("node");
    command.arg("server.js").current_dir(repo_root);
    // Hand our own vault argument to the child, so the desktop shell resolves
    // the vault in the same order as the other two front ends. The child runs
    // with repo_root as its working directory, so a relative path has to be
    // made absolute here -- otherwise it would resolve against the repository
    // rather than the directory the user typed the command in.
    if let Some(vault) = std::env::args_os().nth(1) {
        match std::env::current_dir() {
            Ok(cwd) => command.arg(cwd.join(vault)),
            Err(_) => command.arg(vault),
        };
    }
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let mut child: Child = command
        .spawn()
        .expect("failed to spawn `node server.js` -- is Node.js on PATH?");

    // ponytail: a fixed poll of 5 seconds with a 100 ms step. There is no
    // setting for it. Make the values larger if a slow machine needs more time.
    if !wait_for_port(3000, Duration::from_secs(5)) {
        let _ = child.kill();
        eprintln!(
            "usher: server.js did not open port 3000 within 5s -- it likely exited early.\n\
             Check that the vault exists (default: ../obsidian, or set USHER_VAULT)."
        );
        std::process::exit(1);
    }

    let mut child = Some(child);

    tauri::Builder::default()
        .build(tauri::generate_context!())
        .expect("error building tauri application")
        .run(move |_app_handle, event| {
            if let tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit = event {
                if let Some(mut c) = child.take() {
                    let _ = c.kill();
                }
            }
        });
}
