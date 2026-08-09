# usher (TUI)

A terminal kanban board for an Obsidian "Ticket Vault" (Markdown + YAML frontmatter).
It has the same edit functions as the web UI.
It runs natively on Windows and on Linux/WSL.
The vault must follow the Ticket Vault structure from
[y4m3/obsidian-template](https://github.com/y4m3/obsidian-template).
See the root [`README.md`](../README.md) for the format.

<p align="center">
  <img src="../assets/tui.svg" width="700" alt="The TUI kanban board">
</p>

## Usage

```
usher [vault-root]
```

usher finds the vault in this order:

1. The CLI argument.
2. The `USHER_VAULT` environment variable.
3. The current directory.

The vault root must contain a `tickets/` folder.
If it does not, usher shows the usage text and exits with a non-zero code.

### Install as a command (Windows)

```powershell
cargo install --path .                   # usher.exe → ~\.cargo\bin (on PATH)
setx USHER_VAULT C:\path\to\your\vault   # applies to new shells
usher                                    # start from anywhere
```

## Keys

| Key | Function |
| --- | -------- |
| `←` `→` `h` `l` | Move between the open / doing / review / done columns |
| `↑` `↓` `j` `k` | Move the card selection |
| `n` | Make a new ticket (title prompt; edit the other fields after) |
| `e` | Edit the selected ticket (field picker: title / priority / due / project / tags) |
| `s` | Change the status (popup picker) |
| `m` | Add a `## Log` note |
| `/` | Search by id or title |
| `p` / `t` | Filter by project / tag |
| `.` | Switch the done column between last-7-days and all |
| `Enter` | Open the detail view |
| `r` | Reload from disk (also automatic, each 30 seconds) |
| `q` | Quit |

The detail view shows the ticket fields and sections with formatting.
In the detail view:

- `j` / `k` — scroll.
- `Enter` or `e` — open the full ticket in `$EDITOR` as plain Markdown.
- `s` / `m` — change the status / add a Log note. The view shows the result.
- `Esc` or `q` — close the view.

On a narrow terminal, the board shows fewer, wider columns.
The focused column always stays on screen.
An arrow (`◂` / `▸`) in a column title points to hidden columns.

## The editor

A full-ticket edit stops the TUI and opens `$EDITOR`.
If `$EDITOR` is not set, usher opens `notepad` on Windows and `vi` on other systems.
Save the file and close the editor to apply the edit.
usher lints the result against the vault schema before it writes.
An invalid edit does not touch the file.
To discard an edit, make the editor exit with a non-zero code (in vim: `:cq`).

`archived` tickets do not show on the board.
The `s` popup can still move a ticket to `archived`.

## Build

### Windows (native)

```powershell
cargo test
cargo build --release    # target\release\usher.exe
```

Or use `cargo install --path .` as shown above.

### WSL / Linux

A build on `/mnt/c` is slow. Put the build artifacts on the WSL filesystem:

```
export CARGO_TARGET_DIR=~/.cache/task-app-tui-target
cargo test
cargo build --release
```

The binary is at `~/.cache/task-app-tui-target/release/usher`.

## Layout

- `src/vault.rs` — all vault read and write logic, with no UI dependency. The tests are here.
- `src/ui.rs` — the rendering and the "Tracer" color theme.
- `src/main.rs` — the CLI, the application state, and the key handling.

## Writes

Each write only touches the lines that must change.
All other bytes stay as they are, the line-ending style included.
Before a write, usher lints the new content with a Rust port of the vault's `check_vault.js` rules.
After a write, usher reads the file back and compares the bytes.
On a mismatch, usher restores the original content.
`## Log` is append-only. New tickets copy the vault template byte for byte.
