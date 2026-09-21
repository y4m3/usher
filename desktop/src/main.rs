// Start node server.js and show the web UI in a WebView window.
// This shell uses no sidecar plugin and no shell plugin. A plain
// std::process::Command is sufficient.
//
// A GUI shell has no console. Without the attribute below, Windows gives the
// shell a console. The console then stays on the screen behind the window.
// A debug build keeps the console, because the log messages go to it.
//
// The attribute has a cost: a release build has no console, so eprintln!
// alone is invisible to a user who starts the exe from Explorer or a
// shortcut. `fail` below also shows startup errors in a Win32 message box.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command};
use std::time::{Duration, Instant};
#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// CREATE_NO_WINDOW. A console subsystem child (node) opens its own console
/// window if the parent has no console. This flag stops that window.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The port for server.js. The window url in tauri.conf.json contains the
/// same number.
const PORT: u16 = 3000;

/// Wait until server.js listens on `port`. The error is the message to print.
/// The caller adds nothing to it and makes no decision from it.
fn wait_for_port(child: &mut Child, port: u16, timeout: Duration) -> Result<(), String> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        // Call try_wait() first. If the child stopped, a different process
        // can hold the port (see the comment at the bind below). A successful
        // connection then points the WebView at that process. The shell must
        // report the stop of the child instead.
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

/// The path of the real Node binary. A version manager (mise, volta, fnm,
/// nvm-windows) puts a shim named `node` on PATH. On Windows the shim does
/// not exec: it starts the real node as a grandchild. `kill()` on the shim
/// then leaves node running with the port, `try_wait()` watches the wrong
/// process, and the grandchild opens its own console window. Asking node
/// for its own path once makes the real binary the direct child.
/// Falls back to "node" so the spawn error message below still applies.
///
/// ponytail: this resolver call itself still goes through the shim, so on
/// Windows a console may flash briefly once at startup. A Win32 job object
/// would remove even that; add one if the flash becomes a problem.
fn node_exe() -> String {
    let mut command = Command::new("node");
    command.args(["-p", "process.execPath"]);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    match command.output() {
        Ok(out) if out.status.success() => String::from_utf8(out.stdout).ok(),
        _ => None,
    }
    .map(|p| p.trim().to_string())
    .filter(|p| !p.is_empty())
    .unwrap_or_else(|| "node".to_string())
}

/// Report a startup error and exit. A release build has no console, so
/// eprintln! alone is invisible. On Windows also show a message box.
fn fail(msg: &str) -> ! {
    eprintln!("{msg}");
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "user32")]
        extern "system" {
            fn MessageBoxW(hwnd: *mut std::ffi::c_void, text: *const u16, caption: *const u16, utype: u32) -> i32;
        }
        let wide = |s: &str| std::ffi::OsStr::new(s).encode_wide().chain(Some(0)).collect::<Vec<u16>>();
        let (text, caption) = (wide(msg), wide("usher"));
        const MB_ICONERROR: u32 = 0x10;
        // SAFETY: both buffers are NUL-terminated and outlive the call.
        unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), MB_ICONERROR) };
    }
    std::process::exit(1);
}

fn main() {
    // server.js is at the repository root, one directory above this crate.
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("desktop/ has a parent directory");

    // Bind the port here, before the shell starts the child. This tells two
    // conditions apart: a different process listens on the port already, and
    // the child did not open the port yet. If a different process holds the
    // port, the child cannot bind it. The child then stops with EADDRINUSE,
    // or the shell shows the pages of that other process in the WebView.
    // Release the port immediately, because the child must bind it.
    // ponytail: a gap stays between the release of this listener and the
    // start of the child. A different process can take the port in that gap.
    // To remove the gap, give the bound socket to the child. The child must
    // then not bind its own socket.
    match TcpListener::bind(("127.0.0.1", PORT)) {
        Ok(listener) => drop(listener),
        Err(err) => fail(&format!(
            "usher: port {PORT} is already in use ({err}).\n\
             Another usher instance (or something else) is already listening on \
             127.0.0.1:{PORT}. Stop it and try again."
        )),
    }

    // Resolve the vault path the same way server.js does: the first CLI
    // argument (made absolute against the current directory), else
    // USHER_VAULT or ../obsidian, both relative to the repository like
    // server.js does (it resolves against __dirname, not its cwd). Computed
    // once so the pre-flight check below and the argument passed to the
    // child agree. The working directory of the child is repo_root. Thus
    // make a relative CLI path absolute here. If you do not, the child
    // finds the path from the repository, and not from the directory of
    // the user.
    let cli_vault = std::env::args_os().nth(1).map(|vault| match std::env::current_dir() {
        Ok(cwd) => cwd.join(vault),
        Err(_) => std::path::PathBuf::from(vault),
    });
    let vault_path = cli_vault
        .clone()
        .unwrap_or_else(|| repo_root.join(std::env::var_os("USHER_VAULT").unwrap_or_else(|| "../obsidian".into())));
    if !vault_path.join("system/scripts/check_vault.js").is_file() {
        fail(&format!(
            "usher: no vault at {}\n\
             (system/scripts/check_vault.js is missing there).\n\n\
             Pass the vault path as the first argument, or set USHER_VAULT.\n\
             The default is ../obsidian next to the usher repository.",
            vault_path.display()
        ));
    }

    let mut command = Command::new(node_exe());
    command.arg("server.js").current_dir(repo_root);
    // Give the vault argument of the shell to the child. The desktop shell
    // then finds the vault in the same order as the other two front ends.
    // When no argument was given, pass nothing through: server.js resolves
    // USHER_VAULT and the default itself.
    if let Some(vault) = cli_vault {
        command.arg(vault);
    }
    // server.js reads the PORT variable. The url in tauri.conf.json and
    // wait_for_port below always use 3000. Thus set PORT for the child, and
    // ignore the PORT variable of the parent environment.
    command.env("PORT", PORT.to_string());
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let mut child: Child = command.spawn().unwrap_or_else(|err| {
        fail(&format!(
            "usher: failed to spawn node server.js ({err}). Is Node.js installed and on PATH?"
        ))
    });

    // ponytail: a fixed poll of 5 seconds with a 100 ms step. There is no
    // setting for it. Make the values larger if a slow machine needs more time.
    if let Err(msg) = wait_for_port(&mut child, PORT, Duration::from_secs(5)) {
        fail(&format!(
            "{msg}\nCheck that the vault exists (default: ../obsidian, or set USHER_VAULT)."
        ));
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
