// Start node server.js and show the web UI in a WebView window.
// This shell uses no sidecar plugin and no shell plugin. A plain
// std::process::Command is sufficient.
//
// A GUI shell has no console. Without this, Windows attaches one and it stays
// on screen behind the window. Debug builds keep the console for the logs.
//
// ponytail: the cost is that the startup diagnostics below only reach someone
// who started the exe from a terminal. Launched from Explorer or a shortcut, a
// release build just fails to open a window. Showing the reason there needs a
// Win32 MessageBoxW or a dialog crate; add one if this bites.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command};
use std::time::{Duration, Instant};
#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// CREATE_NO_WINDOW. A console subsystem child (node) opens its own console
/// window when the parent has none. This flag suppresses it.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Fixed port for server.js; also hardcoded in tauri.conf.json's window url.
const PORT: u16 = 3000;

/// Wait until server.js listens on `port`. The error is the line to print:
/// the caller has nothing to add and nothing to decide.
fn wait_for_port(child: &mut Child, port: u16, timeout: Duration) -> Result<(), String> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        // Ask try_wait() first. If the child is already dead, some other
        // process can hold the port (see the bind comment below). A connect
        // that finds that process would point the WebView at it, instead of
        // a report of the crash.
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!(
                "usher: server.js exited before opening port {port} (status: {status})."
            ));
        }
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    Err(format!(
        "usher: server.js did not open port {port} within {}s -- it likely exited early.",
        timeout.as_secs()
    ))
}

fn main() {
    // server.js is at the repository root, one directory above this crate.
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("desktop/ has a parent directory");

    // Claim the port ourselves first so we can tell "someone else is already
    // listening on it" apart from "our own child hasn't opened it yet". If
    // another usher instance (or anything else) already holds the port, the
    // child would fail to bind it and either die (EADDRINUSE) or, worse, we'd
    // end up pointing the WebView at that unrelated process. Release it
    // immediately so the child can bind it in turn.
    // ponytail: the gap between dropping this listener and spawning the
    // child remains -- another process can still grab the port in between.
    // Closing it for good would mean passing the bound socket to the child
    // instead of letting it bind its own.
    match TcpListener::bind(("127.0.0.1", PORT)) {
        Ok(listener) => drop(listener),
        Err(err) => {
            eprintln!(
                "usher: port {PORT} is already in use ({err}).\n\
                 Another usher instance (or something else) is already listening on \
                 127.0.0.1:{PORT}. Stop it and try again."
            );
            std::process::exit(1);
        }
    }

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
    // server.js honours PORT; the shell's URL (tauri.conf.json) and wait_for_port
    // below are fixed at 3000, so pin the child to that port regardless of the
    // parent environment's PORT.
    command.env("PORT", PORT.to_string());
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let mut child: Child = command
        .spawn()
        .expect("failed to spawn `node server.js` -- is Node.js on PATH?");

    // ponytail: a fixed poll of 5 seconds with a 100 ms step. There is no
    // setting for it. Make the values larger if a slow machine needs more time.
    if let Err(msg) = wait_for_port(&mut child, PORT, Duration::from_secs(5)) {
        eprintln!("{msg}\nCheck that the vault exists (default: ../obsidian, or set USHER_VAULT).");
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
