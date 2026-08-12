// Validates every ticket's frontmatter against the schema in AGENTS.md.
// Usage: node system/scripts/check_vault.js [vault-root]   (exit 1 on problems)
// Lives in the Templater user-scripts folder, so it also exports a function.
const fs = require("fs");
const path = require("path");

const STATUS = ["open", "doing", "review", "done", "archived"];
const PRIORITY = ["urgent", "high", "normal", "low"];
const DATE = /^\d{4}-\d{2}-\d{2}$/;
// The closing "---" must end the line (only trailing spaces/tabs allowed), or
// text like "---oops" would be accepted as the terminator.
const FRONTMATTER_RE = /^---\r?\n([\s\S]*?)\r?\n---(?=[ \t]*\r?\n|[ \t]*$)/;

// The escapes this vault actually needs from a YAML double-quoted scalar.
const YAML_ESCAPES = { "\\": "\\", '"': '"', "/": "/", n: "\n", t: "\t", r: "\r", 0: "\0" };

// Resolve the escapes inside the (already unwrapped) body of a double-quoted
// scalar. An escape outside the table above is unknown: leave the backslash
// in place rather than guess, so the value is never silently corrupted.
//
// \uD800-\uDFFF (the surrogate range) is also left as-is. This format does
// not compose surrogate pairs, so a surrogate \uXXXX is an unknown escape. An
// astral character (above U+FFFF) still works when it is written literally in
// the file as UTF-8 — this only affects the \uXXXX escape form.
function unescapeYaml(body) {
  return body.replace(/\\(?:u([0-9a-fA-F]{4})|x([0-9a-fA-F]{2})|(.))/g, (full, u, x, ch) => {
    if (u !== undefined) {
      const code = parseInt(u, 16);
      return code >= 0xd800 && code <= 0xdfff ? full : String.fromCodePoint(code);
    }
    if (x !== undefined) return String.fromCodePoint(parseInt(x, 16));
    return Object.hasOwn(YAML_ESCAPES, ch) ? YAML_ESCAPES[ch] : full;
  });
}

// True if the body of a double-quoted scalar contains an escape outside the
// table above (`unescapeYaml` would leave it untouched, backslash and all),
// including a \uXXXX in the surrogate range (see unescapeYaml).
function hasUnknownEscape(body) {
  let unknown = false;
  body.replace(/\\(?:u([0-9a-fA-F]{4})|x[0-9a-fA-F]{2}|(.))/g, (full, u, ch) => {
    if (u !== undefined) {
      const code = parseInt(u, 16);
      if (code >= 0xd800 && code <= 0xdfff) unknown = true;
    } else if (ch !== undefined && !Object.hasOwn(YAML_ESCAPES, ch)) unknown = true;
    return full;
  });
  return unknown;
}

// Obsidian's YAML parser accepts quoted scalars everywhere, so strip
// well-formed surrounding quotes before validating — `id: "T-0042"` is valid.
function unquote(v) {
  if (v == null) return v;
  let m = /^"((?:[^"\\]|\\.)*)"$/.exec(v);
  if (m) return unescapeYaml(m[1]);
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
    const m = text.match(FRONTMATTER_RE);
    if (!m) { err("missing frontmatter"); continue; }
    const fm = {};
    const seenKeys = new Set();
    for (const line of m[1].split(/\r?\n/)) {
      const kv = line.match(/^(\w+):\s*(.*)$/);
      if (!kv) continue;
      // First key wins, but a duplicate is a schema violation, not something
      // to quietly resolve.
      if (seenKeys.has(kv[1])) err(`duplicate frontmatter key "${kv[1]}"`);
      seenKeys.add(kv[1]);
      if (!(kv[1] in fm)) fm[kv[1]] = kv[2].trim();
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
      const dq = /^"((?:[^"\\]|\\.)*)"$/.exec(title);
      if (!dq && !/^'[^']*'$/.test(title)) err(`malformed quoted title ${title}`);
      else if (dq && hasUnknownEscape(dq[1])) err(`unknown escape in title ${title}`);
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
    // Every field keeps its line, even with an empty value. id, title, status,
    // priority and created are covered above by their own value checks; these
    // six can legitimately be empty, so check that the line is there. A tool
    // that edits a ticket replaces a field line in its position and cannot add
    // one, which is why an absent line is an error and not an empty value.
    for (const k of ["project", "repos", "tags", "due", "closed", "branch"]) {
      if (fm[k] === undefined) err(`missing ${k}`);
    }
    for (const k of ["due", "closed"]) {
      const v = unquote(fm[k]);
      if (v && !DATE.test(v)) err(`bad ${k} "${v}" (want YYYY-MM-DD)`);
    }
    if (status === "done" && !unquote(fm.closed)) err("status done but closed is empty");
    if (unquote(fm.closed) && status !== "done" && status !== "archived")
      err(`closed is set but status is "${status}" (want done or archived)`);
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
