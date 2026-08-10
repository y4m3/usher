// Validates every ticket's frontmatter against the schema in AGENTS.md.
// Usage: node system/scripts/check_vault.js [vault-root]   (exit 1 on problems)
// Lives in the Templater user-scripts folder, so it also exports a function.
const fs = require("fs");
const path = require("path");

const STATUS = ["open", "doing", "review", "done", "archived"];
const PRIORITY = ["urgent", "high", "normal", "low"];
const DATE = /^\d{4}-\d{2}-\d{2}$/;

// Obsidian's YAML parser accepts quoted scalars everywhere, so strip
// well-formed surrounding quotes before validating — `id: "T-0042"` is valid.
function unquote(v) {
  if (v == null) return v;
  let m = /^"((?:[^"\\]|\\.)*)"$/.exec(v);
  if (m) return m[1].replace(/\\(.)/g, "$1");
  m = /^'([^']*)'$/.exec(v);
  return m ? m[1] : v;
}

function checkVault(root) {
  const dir = path.join(root, "tickets");
  if (!fs.existsSync(dir))
    return [`no tickets/ folder in ${root} — run from the vault root`];
  const errors = [];
  const ids = new Map();
  const entries = fs
    .readdirSync(dir, { recursive: true })
    .map(String)
    .filter((n) => n.endsWith(".md"));
  for (const name of entries) {
    const err = (m) => errors.push(`${name}: ${m}`);
    const text = fs.readFileSync(path.join(dir, name), "utf8");
    const m = text.match(/^---\r?\n([\s\S]*?)\r?\n---/);
    if (!m) { err("missing frontmatter"); continue; }
    const fm = {};
    for (const line of m[1].split(/\r?\n/)) {
      const kv = line.match(/^(\w+):\s*(.*)$/);
      if (kv) fm[kv[1]] = kv[2].trim();
    }
    const id = unquote(fm.id) || "";
    if (!/^T-\d{4}$/.test(id))
      err(fm.id === undefined ? "missing id" : `bad id "${id}" (want T-NNNN, 4 digits)`);
    else {
      if (ids.has(id)) err(`duplicate id ${id} (also in ${ids.get(id)})`);
      ids.set(id, name);
      if (!path.basename(name).startsWith(id)) err(`filename does not start with ${id}`);
    }
    // Title is checked on the RAW value — quoting correctness is the point
    // here. Obsidian's properties UI strips quotes it deems unnecessary, so
    // accept unquoted titles; flag only what actually breaks YAML — an
    // unquoted ":" or a malformed/empty quoted string.
    const title = fm.title || "";
    if (!title || title === '""' || title === "''") err("title missing");
    else if (/^["']/.test(title)) {
      if (!/^("([^"\\]|\\.)*"|'[^']*')$/.test(title)) err(`malformed quoted title ${title}`);
    } else if (title.includes(":")) err("title with ':' must be quoted");
    const status = unquote(fm.status);
    if (!STATUS.includes(status))
      err(status === undefined ? "missing status" : `bad status "${status}"`);
    const priority = unquote(fm.priority);
    if (!PRIORITY.includes(priority))
      err(priority === undefined ? "missing priority" : `bad priority "${priority}"`);
    const created = unquote(fm.created);
    if (created === undefined || created === "") err("missing created");
    else if (!DATE.test(created)) err(`bad created "${created}" (want YYYY-MM-DD)`);
    for (const k of ["due", "closed"]) {
      const v = unquote(fm[k]);
      if (v && !DATE.test(v)) err(`bad ${k} "${v}" (want YYYY-MM-DD)`);
    }
    if (status === "done" && !unquote(fm.closed)) err("status done but closed is empty");
    const branch = unquote(fm.branch);
    if (branch && !branch.startsWith(id)) err(`branch "${branch}" does not start with ${id}`);
  }
  return errors;
}

if (require.main === module) {
  const errors = checkVault(process.argv[2] || process.cwd());
  for (const e of errors) console.error(e);
  console.log(errors.length ? `FAIL: ${errors.length} problem(s)` : "OK");
  process.exit(errors.length ? 1 : 0);
}
module.exports = checkVault;
