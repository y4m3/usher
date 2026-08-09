// Start node server.js and show the web UI in a WebView window.
// This shell uses no sidecar plugin and no shell plugin. A plain
// std::process::Command is sufficient.
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

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

    let child: Child = Command::new("node")
        .arg("server.js")
        .current_dir(repo_root)
        .spawn()
        .expect("failed to spawn `node server.js` -- is Node.js on PATH?");

    // ponytail: a fixed poll of 5 seconds with a 100 ms step. There is no
    // setting for it. Make the values larger if a slow machine needs more time.
    wait_for_port(3000, Duration::from_secs(5));

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
