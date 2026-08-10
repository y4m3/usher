// Minimal HTTP server for usher, a kanban board over the Obsidian Ticket Vault.
// It has no dependencies.
// Usage: node server.js [vault-root]   (default vault-root: ../obsidian)
// Vault resolution order: CLI argument, then USHER_VAULT, then the default.
// Same order as the TUI (see tui/README.md).
"use strict";
const http = require("node:http");
const fs = require("node:fs");
const path = require("node:path");

const vaultRoot = path.resolve(
  __dirname,
  process.argv[2] || process.env.USHER_VAULT || "../obsidian",
);
const ticketsDir = path.join(vaultRoot, "tickets");
const projectsDir = path.join(vaultRoot, "projects");
const checkVault = require(path.join(vaultRoot, "system/scripts/check_vault.js"));

const STATUS = ["open", "doing", "review", "done", "archived"];
const PRIORITY = ["urgent", "high", "normal", "low"];
const ID_RE = /^T-\d{4}$/;
const DATE_RE = /^\d{4}-\d{2}-\d{2}$/;
const TAG_RE = /^[A-Za-z0-9_][A-Za-z0-9_/-]*$/;
// The closing "---" must end the line (only trailing spaces/tabs allowed), or
// text like "---oops" would be accepted as the terminator. The lookahead does
// not consume the newline, so `m[0]` still ends right after "---", same as
// before.
const FRONTMATTER_RE = /^---\r?\n([\s\S]*?)\r?\n---(?=[ \t]*\r?\n|[ \t]*$)/;

// The escapes this vault actually needs from a YAML double-quoted scalar.
// Kept identical in check_vault.js, which validates against the same table.
const YAML_ESCAPES = { "\\": "\\", '"': '"', "/": "/", n: "\n", t: "\t", r: "\r", 0: "\0" };

// Resolve the escapes inside the (already unwrapped) body of a double-quoted
// scalar. An escape outside the table above is unknown: leave the backslash
// in place rather than guess, so the value is never silently corrupted.
// check_vault.js flags what this leaves behind.
//
// \uD800-\uDFFF (the surrogate range) is also left as-is: Rust's
// char::from_u32 rejects lone surrogates and does not compose surrogate
// pairs, so treating a surrogate \uXXXX as unknown is the only reading both
// front ends agree on. An astral character (above U+FFFF) still works fine
// written literally in the file as UTF-8 — this only affects the \uXXXX
// escape form.
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

// The same rule as check_vault.js: remove correct quotes around the value.
function unquote(v) {
  if (v == null) return "";
  let m = /^"((?:[^"\\]|\\.)*)"$/.exec(v);
  if (m) return unescapeYaml(m[1]);
  m = /^'([^']*)'$/.exec(v);
  return m ? m[1] : v;
}

function unwikilink(v) {
  const m = /^\[\[(.*)\]\]$/.exec(v);
  return m ? m[1] : v;
}

function parseFrontmatter(text) {
  const m = text.match(FRONTMATTER_RE);
  if (!m) return null;
  const fm = {};
  for (const line of m[1].split(/\r?\n/)) {
    const kv = line.match(/^(\w+):\s*(.*)$/);
    // First key wins on a duplicate, same as the TUI (Rust side) parser.
    if (kv && !(kv[1] in fm)) fm[kv[1]] = kv[2].trim();
  }
  return fm;
}

// Read only. The regex in parseFrontmatter reads one line for each key. Thus it
// does not read a `tags:` block list (indented `  - item` lines). This function
// scans for them. It reads both `tags: []` and a block list.
function parseTags(text) {
  const m = text.match(FRONTMATTER_RE);
  if (!m) return [];
  const lines = m[1].split(/\r?\n/);
  const idx = lines.findIndex((l) => /^tags:/.test(l));
  if (idx === -1) return [];
  const tags = [];
  for (let i = idx + 1; i < lines.length; i++) {
    const item = lines[i].match(/^\s+-\s*(.+)$/);
    if (!item) break;
    tags.push(unquote(item[1].trim()));
  }
  return tags;
}

function listTickets() {
  const names = fs.readdirSync(ticketsDir).filter((n) => n.endsWith(".md"));
  return names.map((name) => {
    const text = fs.readFileSync(path.join(ticketsDir, name), "utf8");
    const fm = parseFrontmatter(text) || {};
    return {
      id: unquote(fm.id),
      title: unquote(fm.title),
      status: unquote(fm.status),
      priority: unquote(fm.priority),
      due: unquote(fm.due),
      closed: unquote(fm.closed),
      project: unwikilink(unquote(fm.project)),
      tags: parseTags(text),
      file: "tickets/" + name,
    };
  });
}

function findTicketFile(id) {
  const name = fs.readdirSync(ticketsDir).find((n) => n.startsWith(id) && n.endsWith(".md"));
  return name ? path.join(ticketsDir, name) : null;
}

// A project note is either projects/<name>.md, or projects/<name>/<name>.md
// when the vault uses one folder per project (an Obsidian folder note). Only
// the top level is scanned, so the theme folders that may live inside a
// project folder do not end up in the list.
function listProjects() {
  if (!fs.existsSync(projectsDir)) return [];
  return fs.readdirSync(projectsDir, { withFileTypes: true }).flatMap((entry) => {
    if (entry.isFile() && entry.name.endsWith(".md")) return [entry.name.slice(0, -3)];
    if (entry.isDirectory()) {
      const note = fs.statSync(path.join(projectsDir, entry.name, `${entry.name}.md`), { throwIfNoEntry: false });
      if (note && note.isFile()) return [entry.name];
    }
    return [];
  }).sort();
}

// Checked against the list, not against a built path, so that a name with a
// separator or ".." in it can never resolve to a file outside projects/.
function projectExists(name) {
  return listProjects().includes(name);
}

function detectEOL(text) {
  return text.includes("\r\n") ? "\r\n" : "\n";
}

function pad2(n) {
  return String(n).padStart(2, "0");
}

function todayDate() {
  const d = new Date();
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`;
}

function nowStamp() {
  const d = new Date();
  return `${todayDate()} ${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
}

// Replace the value of one `key: value` line in the frontmatter block. Do not
// change any other byte in the file.
function setFrontmatterField(text, key, value) {
  const m = text.match(FRONTMATTER_RE);
  if (!m) throw new Error("missing frontmatter");
  const block = m[0];
  const re = new RegExp(`^${key}:[^\\r\\n]*`, "m");
  if (!re.test(block)) throw new Error(`missing ${key} field`);
  // A function replacement, so a `$&`/`$1`/etc. in value is never read as a
  // regex replacement pattern.
  const newBlock = block.replace(re, () => `${key}: ${value}`);
  return text.slice(0, m.index) + newBlock + text.slice(m.index + block.length);
}

// Replace the full `tags:` block with a new one. The block is the `tags:` line
// and its `  - item` lines. Do not change any other line, in the frontmatter or
// in the body.
function setTagsBlock(text, tags) {
  const fm = text.match(FRONTMATTER_RE);
  if (!fm) throw new Error("missing frontmatter");
  const block = text.slice(0, fm.index + fm[0].length);
  const m = block.match(/^tags:[^\r\n]*\r?\n(?:[ \t]+-[^\r\n]*\r?\n)*/m);
  if (!m) throw new Error("missing tags field");
  const eol = detectEOL(text);
  const newBlock = tags.length ? `tags:${eol}${tags.map((t) => `  - ${t}${eol}`).join("")}` : `tags: []${eol}`;
  return text.slice(0, m.index) + newBlock + text.slice(m.index + m[0].length);
}

// Find the body of a `## <header>` section. The body starts after the header
// line. It stops at the next `## ` heading or at the end of the file. Every
// section function below uses this one. Thus the rule is in one place.
function sectionBounds(text, header) {
  const m = text.match(new RegExp(`^## ${header}\\r?\\n`, "m"));
  if (!m) throw new Error(`missing ## ${header} section`);
  const start = m.index + m[0].length;
  const rest = text.slice(start);
  const next = rest.match(/^## /m);
  const end = start + (next ? next.index : rest.length);
  return { start, end };
}

// Add one line at the end of the `## Log` section. Do not change an existing
// line.
function appendLog(text, line) {
  const { start, end } = sectionBounds(text, "Log");
  const section = text.slice(start, end);
  const trimmed = section.replace(/\s+$/, "");
  const trailingWS = section.slice(trimmed.length);
  const eol = detectEOL(text);
  const newSection = trimmed + eol + line + trailingWS;
  return text.slice(0, start) + newSection + text.slice(end);
}

// Read the content of a section, without the empty lines around it. A write puts
// those lines back without a change. See setSectionCore.
function getSectionCore(text, header) {
  const { start, end } = sectionBounds(text, header);
  return text.slice(start, end).trim();
}

// Replace only the text of a section. Keep the empty lines before and after the
// text. Keep every byte outside of the section.
function setSectionCore(text, header, newCore) {
  const { start, end } = sectionBounds(text, header);
  const section = text.slice(start, end);
  const eol = detectEOL(text);
  const normalized = newCore.replace(/\r\n|\n/g, eol);
  if (section.trim() === "") {
    // The section has only whitespace. A previous write can clear it. Then ^\s*
    // and \s*$ both match the full section. leadingWS and trailingWS would keep
    // these lines two times, and the section would grow on each write. Use a
    // fixed padding instead: one empty line after the header and one empty line
    // before the next header. The ticket template uses the same spacing.
    return text.slice(0, start) + eol + normalized + eol + eol + text.slice(end);
  }
  const leadingWS = section.match(/^\s*/)[0];
  const trailingWS = section.match(/\s*$/)[0];
  return text.slice(0, start) + leadingWS + normalized + trailingWS + text.slice(end);
}

function getSections(text) {
  return { summary: getSectionCore(text, "Summary"), notes: getSectionCore(text, "Notes") };
}

// The same YAML string escaping that the Templater ticket template uses.
function quoteYaml(title) {
  return `"${title.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
}

// The same slug rule as the Templater ticket template: lowercase ASCII
// kebab-case, with a maximum of 40 characters.
function slugify(title) {
  return title
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 40);
}

// The same rule as system/scripts/next_ticket_id.js. Take the highest number
// from the frontmatter id and from the file name of every ticket. Add 1. Then
// pad the number to 4 digits.
function nextTicketId() {
  let max = 0;
  for (const name of fs.readdirSync(ticketsDir).filter((n) => n.endsWith(".md"))) {
    const fm = parseFrontmatter(fs.readFileSync(path.join(ticketsDir, name), "utf8")) || {};
    for (const candidate of [unquote(fm.id), name]) {
      const m = /^T-(\d+)/.exec(candidate);
      if (m) max = Math.max(max, parseInt(m[1], 10));
    }
  }
  if (max + 1 > 9999) throw new Error("ticket id space exhausted (T-9999 cap)");
  return "T-" + String(max + 1).padStart(4, "0");
}

// The frontmatter and the body for a new ticket. It matches
// system/templates/ticket.md byte for byte in style: key order, quoting and
// empty-field spacing.
function newTicketContent(id, title, priority, due, project, basename) {
  return [
    "---",
    `id: ${id}`,
    `title: ${quoteYaml(title)}`,
    "status: open",
    `priority: ${priority}`,
    project ? `project: "[[${project}]]"` : "project: ",
    "repos: []",
    "tags: []",
    `created: ${todayDate()}`,
    due ? `due: ${due}` : "due: ",
    "closed: ",
    `branch: ${basename}`,
    "---",
    "",
    "## Summary",
    "",
    title,
    "",
    "## Notes",
    "",
    "- ",
    "",
    "## Log",
    "",
    `- ${nowStamp()} — created`,
    "",
  ].join("\n");
}

function logMessageFor(status, note) {
  switch (status) {
    case "doing":
      return "started";
    case "done":
      return note ? `done: ${note}` : "done";
    case "review":
      return note || "review requested";
    case "archived":
      return note || "archived";
    default:
      return note || `reopened`;
  }
}

// Read the file. For a new ticket the file does not exist yet, and the value is
// null. Apply `mutate(original) -> text`, write the result, then lint the vault
// again. If the lint fails, write the original content back. For a new ticket,
// delete the file. Then throw an Error with a `.problems` property.
function writeTicket(filePath, mutate) {
  const exists = fs.existsSync(filePath);
  const original = exists ? fs.readFileSync(filePath, "utf8") : null;
  const updated = mutate(original);
  fs.writeFileSync(filePath, updated, "utf8");
  // A failed rollback must not hide whatever made us roll back, and it must not
  // pass in silence either: the rejected edit is then still on disk, and this
  // is the only place that can say so. Returns the trouble, or null.
  const rollback = () => {
    try {
      if (exists) fs.writeFileSync(filePath, original, "utf8");
      else fs.unlinkSync(filePath);
      return null;
    } catch (e) {
      return `could not ${exists ? "restore" : "remove"} ${filePath} (${e.message}); the rejected write is still on disk`;
    }
  };
  let problems;
  try {
    problems = checkVault(vaultRoot);
  } catch (e) {
    const stuck = rollback();
    if (stuck) e.message = `${e.message} -- ${stuck}`;
    throw e;
  }
  if (problems.length) {
    const stuck = rollback();
    const err = new Error(stuck ? `lint failed -- ${stuck}` : "lint failed");
    err.problems = problems;
    throw err;
  }
}

function sendJSON(res, status, body) {
  const data = JSON.stringify(body);
  res.writeHead(status, { "Content-Type": "application/json" });
  res.end(data);
}

function readBody(req) {
  return new Promise((resolve, reject) => {
    let data = "";
    req.on("data", (chunk) => (data += chunk));
    req.on("end", () => {
      if (!data) return resolve({});
      try {
        resolve(JSON.parse(data));
      } catch (e) {
        reject(e);
      }
    });
    req.on("error", reject);
  });
}

// Return the path of the ticket file. If the id is bad, send 400 and return
// null. If the ticket does not exist, send 404 and return null.
function resolveTicket(id, res) {
  if (!ID_RE.test(id)) {
    sendJSON(res, 400, { error: "bad ticket id" });
    return null;
  }
  const file = findTicketFile(id);
  if (!file) sendJSON(res, 404, { error: "ticket not found" });
  return file;
}

const server = http.createServer(async (req, res) => {
  const url = new URL(req.url, "http://localhost");
  const pathname = url.pathname;

  try {
    if (req.method === "GET" && pathname === "/api/tickets") {
      return sendJSON(res, 200, listTickets());
    }

    if (req.method === "GET" && pathname === "/api/projects") {
      return sendJSON(res, 200, listProjects());
    }

    if (req.method === "POST" && pathname === "/api/tickets") {
      const body = await readBody(req);
      const title = typeof body.title === "string" ? body.title.trim() : "";
      if (!title) return sendJSON(res, 400, { error: "title required" });
      const priority = body.priority ?? "normal";
      if (!PRIORITY.includes(priority)) return sendJSON(res, 400, { error: "bad priority" });
      const due = body.due ? String(body.due) : "";
      if (due && !DATE_RE.test(due)) return sendJSON(res, 400, { error: "bad due (want YYYY-MM-DD)" });
      const project = body.project ? String(body.project) : "";
      if (project && !projectExists(project)) return sendJSON(res, 400, { error: "unknown project" });
      let id;
      try {
        id = nextTicketId();
      } catch (e) {
        return sendJSON(res, 500, { error: e.message });
      }
      const slug = slugify(title);
      const basename = slug ? `${id}-${slug}` : id;
      const file = path.join(ticketsDir, `${basename}.md`);
      try {
        writeTicket(file, () => newTicketContent(id, title, priority, due, project, basename));
      } catch (e) {
        return sendJSON(res, 500, { error: e.message, problems: e.problems || [] });
      }
      return sendJSON(res, 200, { id, file: `tickets/${basename}.md` });
    }

    let m = pathname.match(/^\/api\/tickets\/([^/]+)$/);
    if (m && req.method === "GET") {
      const file = resolveTicket(m[1], res);
      if (!file) return;
      res.writeHead(200, { "Content-Type": "text/plain; charset=utf-8" });
      return res.end(fs.readFileSync(file, "utf8"));
    }

    m = pathname.match(/^\/api\/tickets\/([^/]+)\/status$/);
    if (m && req.method === "POST") {
      const file = resolveTicket(m[1], res);
      if (!file) return;
      const body = await readBody(req);
      if (!STATUS.includes(body.status)) return sendJSON(res, 400, { error: "bad status" });
      const current = parseFrontmatter(fs.readFileSync(file, "utf8")) || {};
      if (unquote(current.status) === body.status) return sendJSON(res, 200, { ok: true, unchanged: true });
      try {
        writeTicket(file, (text) => {
          text = setFrontmatterField(text, "status", body.status);
          if (body.status === "done") text = setFrontmatterField(text, "closed", todayDate());
          // Only the new status decides: moving to anything but done or
          // archived clears closed, moving to archived leaves it alone. That
          // is what keeps closed as the record of when the work finished.
          else if (body.status !== "archived") text = setFrontmatterField(text, "closed", "");
          const msg = logMessageFor(body.status, body.note);
          return appendLog(text, `- ${nowStamp()} — ${msg}`);
        });
      } catch (e) {
        return sendJSON(res, 500, { error: e.message, problems: e.problems || [] });
      }
      return sendJSON(res, 200, { ok: true });
    }

    m = pathname.match(/^\/api\/tickets\/([^/]+)\/log$/);
    if (m && req.method === "POST") {
      const file = resolveTicket(m[1], res);
      if (!file) return;
      const body = await readBody(req);
      const text = typeof body.text === "string" ? body.text.trim() : "";
      if (!text) return sendJSON(res, 400, { error: "text required" });
      try {
        writeTicket(file, (t) => appendLog(t, `- ${nowStamp()} — ${text}`));
      } catch (e) {
        return sendJSON(res, 500, { error: e.message, problems: e.problems || [] });
      }
      return sendJSON(res, 200, { ok: true });
    }

    m = pathname.match(/^\/api\/tickets\/([^/]+)\/fields$/);
    if (m && req.method === "POST") {
      const file = resolveTicket(m[1], res);
      if (!file) return;
      const body = await readBody(req);
      const allowed = ["title", "priority", "due", "project", "tags"];
      const bad = Object.keys(body).filter((k) => !allowed.includes(k));
      if (bad.length) return sendJSON(res, 400, { error: `unknown field(s): ${bad.join(", ")}` });
      if ("title" in body && !String(body.title).trim()) return sendJSON(res, 400, { error: "title required" });
      if ("priority" in body && !PRIORITY.includes(body.priority)) return sendJSON(res, 400, { error: "bad priority" });
      if ("due" in body && body.due && !DATE_RE.test(body.due)) return sendJSON(res, 400, { error: "bad due (want YYYY-MM-DD)" });
      if ("project" in body && body.project && !projectExists(body.project)) return sendJSON(res, 400, { error: "unknown project" });
      if ("tags" in body && (!Array.isArray(body.tags) || !body.tags.every((t) => typeof t === "string" && TAG_RE.test(t)))) {
        return sendJSON(res, 400, { error: "bad tags" });
      }
      try {
        writeTicket(file, (text) => {
          if ("title" in body) text = setFrontmatterField(text, "title", quoteYaml(String(body.title).trim()));
          if ("priority" in body) text = setFrontmatterField(text, "priority", body.priority);
          if ("due" in body) text = setFrontmatterField(text, "due", body.due || "");
          if ("project" in body) text = setFrontmatterField(text, "project", body.project ? `"[[${body.project}]]"` : "");
          if ("tags" in body) text = setTagsBlock(text, [...new Set(body.tags)]);
          return text;
        });
      } catch (e) {
        return sendJSON(res, 500, { error: e.message, problems: e.problems || [] });
      }
      return sendJSON(res, 200, { ok: true });
    }

    m = pathname.match(/^\/api\/tickets\/([^/]+)\/sections$/);
    if (m && req.method === "GET") {
      const file = resolveTicket(m[1], res);
      if (!file) return;
      return sendJSON(res, 200, getSections(fs.readFileSync(file, "utf8")));
    }
    if (m && req.method === "PUT") {
      const file = resolveTicket(m[1], res);
      if (!file) return;
      const body = await readBody(req);
      const allowed = ["summary", "notes"];
      const bad = Object.keys(body).filter((k) => !allowed.includes(k));
      if (bad.length) return sendJSON(res, 400, { error: `unknown field(s): ${bad.join(", ")}` });
      try {
        writeTicket(file, (text) => {
          if ("summary" in body) text = setSectionCore(text, "Summary", String(body.summary ?? ""));
          if ("notes" in body) text = setSectionCore(text, "Notes", String(body.notes ?? ""));
          return text;
        });
      } catch (e) {
        return sendJSON(res, 500, { error: e.message, problems: e.problems || [] });
      }
      return sendJSON(res, 200, { ok: true });
    }

    // public/ has one file. There is nothing else to send.
    if (req.method === "GET" && (pathname === "/" || pathname === "/index.html")) {
      res.writeHead(200, { "Content-Type": "text/html" });
      return res.end(fs.readFileSync(path.join(__dirname, "public/index.html")));
    }

    sendJSON(res, 404, { error: "not found" });
  } catch (e) {
    sendJSON(res, 500, { error: e.message });
  }
});

if (require.main === module) {
  const port = Number(process.env.PORT ?? 3000);
  server.listen(port, "127.0.0.1", () => {
    console.log(`usher listening on http://localhost:${server.address().port} (vault: ${vaultRoot})`);
  });
}

module.exports = server;
