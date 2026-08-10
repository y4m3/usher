// Self-check for server.js. It has no dependencies. It never changes the real
// vault. It copies tickets/ and check_vault.js into a temporary directory and
// runs against that copy.
"use strict";
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawn } = require("node:child_process");

const REAL_VAULT = path.join(__dirname, "tests", "fixtures", "vault");
let BASE; // set after the child server reports the port that the OS gave it

// The vault dates are local dates, like todayDate() in server.js. Do not use
// toISOString(), because it gives the UTC date.
const localISODate = (d) =>
  `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;

function fail(msg) {
  process.exitCode = 1;
  console.error("FAIL -", msg);
}

function assert(cond, msg) {
  if (cond) console.log("ok   -", msg);
  else fail(msg);
}

// LCS diff for lines. The files are small, and O(n*m) is sufficient. It returns
// the lines only in `a` (removed) and the lines only in `b` (added), in file
// order.
function diffLines(a, b) {
  const n = a.length, m = b.length;
  const dp = Array.from({ length: n + 1 }, () => new Array(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i--)
    for (let j = m - 1; j >= 0; j--)
      dp[i][j] = a[i] === b[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
  const removed = [], added = [];
  let i = 0, j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) { i++; j++; }
    else if (dp[i + 1][j] >= dp[i][j + 1]) { removed.push(a[i]); i++; }
    else { added.push(b[j]); j++; }
  }
  while (i < n) removed.push(a[i++]);
  while (j < m) added.push(b[j++]);
  return { removed, added };
}

// Read the "listening on http://localhost:PORT" line of the server. Do not guess
// a port. Thus this test never uses the port of a different process.
function waitForPort(server) {
  return new Promise((resolve, reject) => {
    const deadline = setTimeout(() => reject(new Error("server did not start")), 5000);
    server.stdout.on("data", (d) => {
      const m = /listening on http:\/\/localhost:(\d+)/.exec(d);
      if (m) {
        clearTimeout(deadline);
        resolve(Number(m[1]));
      }
    });
  });
}

function ticketFile(fakeVault, id) {
  const name = fs.readdirSync(path.join(fakeVault, "tickets")).find((n) => n.startsWith(id));
  return path.join(fakeVault, "tickets", name);
}

function currentMaxId(fakeVault) {
  let max = 0;
  for (const name of fs.readdirSync(path.join(fakeVault, "tickets"))) {
    const m = /^T-(\d+)/.exec(name);
    if (m) max = Math.max(max, parseInt(m[1], 10));
  }
  return max;
}

// Copy tickets/ + projects/ + system/scripts/check_vault.js from the real
// fixture vault into `dest`. Shared by main()'s server-backed fakeVault and by
// the throwaway copies the lint tests below break on purpose — the committed
// fixture vault itself must stay lint-clean.
function copyVaultInto(dest) {
  fs.mkdirSync(path.join(dest, "tickets"), { recursive: true });
  fs.mkdirSync(path.join(dest, "projects"), { recursive: true });
  fs.mkdirSync(path.join(dest, "system", "scripts"), { recursive: true });
  fs.cpSync(path.join(REAL_VAULT, "tickets"), path.join(dest, "tickets"), { recursive: true });
  // recursive: projects/ can hold folder notes (projects/<name>/<name>.md), and
  // copyFileSync would EPERM on a directory entry.
  fs.cpSync(path.join(REAL_VAULT, "projects"), path.join(dest, "projects"), { recursive: true });
  fs.copyFileSync(
    path.join(REAL_VAULT, "system", "scripts", "check_vault.js"),
    path.join(dest, "system", "scripts", "check_vault.js")
  );
}

// Copy the vault into a throwaway temp dir, let `breakFn` corrupt it, and
// return the resulting checkVault errors. Used for lint tests that need a
// broken ticket to fail against.
function lintBrokenVault(breakFn) {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "usher-lint-"));
  try {
    copyVaultInto(tmp);
    breakFn(tmp);
    const brokenCheckVault = require(path.join(tmp, "system", "scripts", "check_vault.js"));
    return brokenCheckVault(tmp);
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }
}

async function runTests(fakeVault, checkVault) {
  const list = await fetch(BASE + "/api/tickets").then((r) => r.json());
  assert(list.length === 7, `GET /api/tickets returns 7 tickets, incl. CRLF fixture (got ${list.length})`);
  const t6 = list.find((t) => t.id === "T-0006"); // in the real vault: status done, closed 2026-08-09
  assert(t6 && t6.closed === "2026-08-09", `GET /api/tickets includes closed (got ${JSON.stringify(t6 && t6.closed)})`);
  assert(JSON.stringify(t6 && t6.tags) === JSON.stringify(["launch"]), `tags: block-list parsed (got ${JSON.stringify(t6 && t6.tags)})`);
  const t900 = list.find((t) => t.id === "T-0900"); // the CRLF fixture has tags: []
  assert(t900 && Array.isArray(t900.tags) && t900.tags.length === 0, `tags: [] parsed as empty array (got ${JSON.stringify(t900 && t900.tags)})`);

  // open -> doing: only the status line changes, and one log line is added.
  {
    const file = ticketFile(fakeVault, "T-0002"); // the status is open
    const before = fs.readFileSync(file, "utf8");
    const res = await fetch(BASE + "/api/tickets/T-0002/status", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ status: "doing" }),
    });
    assert(res.ok, "open->doing request succeeds");
    const after = fs.readFileSync(file, "utf8");
    const { removed, added } = diffLines(before.split("\n"), after.split("\n"));
    assert(removed.length === 1 && removed[0].startsWith("status:"), `open->doing: exactly the status line changed (removed=${JSON.stringify(removed)})`);
    assert(added.length === 2 && added[0] === "status: doing", `open->doing: new status line correct (added=${JSON.stringify(added)})`);
    assert(/^- \d{4}-\d{2}-\d{2} \d{2}:\d{2} — started$/.test(added[1]), "open->doing: log line reads 'started'");
    assert(checkVault(fakeVault).length === 0, "open->doing: vault still lints clean");
  }

  // doing -> done: the status and closed lines change, and one log line is
  // added.
  {
    const file = ticketFile(fakeVault, "T-0002"); // the status is now doing
    const before = fs.readFileSync(file, "utf8");
    const res = await fetch(BASE + "/api/tickets/T-0002/status", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ status: "done" }),
    });
    assert(res.ok, "doing->done request succeeds");
    const after = fs.readFileSync(file, "utf8");
    const { removed, added } = diffLines(before.split("\n"), after.split("\n"));
    assert(removed.length === 2, `doing->done: exactly status+closed lines changed (removed=${JSON.stringify(removed)})`);
    const today = localISODate(new Date());
    assert(added.length === 3 && added.includes(`closed: ${today}`), `doing->done: closed set to today + log appended (added=${JSON.stringify(added)})`);
    assert(checkVault(fakeVault).length === 0, "doing->done: vault still lints clean");
  }

  // The same status again changes nothing. There is no write, and the file bytes
  // stay equal.
  {
    const file = ticketFile(fakeVault, "T-0002"); // the status is already done
    const before = fs.readFileSync(file, "utf8");
    const res = await fetch(BASE + "/api/tickets/T-0002/status", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ status: "done" }),
    });
    const body = await res.json();
    assert(res.ok && body.unchanged === true, `same-status POST reports unchanged (got ${JSON.stringify(body)})`);
    const after = fs.readFileSync(file, "utf8");
    assert(after === before, "same-status POST: file byte-identical");
  }

  // Leaving done for anything but archived clears closed. The lint rejects a
  // closed date on a ticket that is neither done nor archived — the lint tests
  // further down cover that rule itself — so a reopen that kept the date would
  // write a ticket the vault refuses. What this block checks is the clearing.
  {
    const file = ticketFile(fakeVault, "T-0002"); // currently done, closed set above
    const res = await fetch(BASE + "/api/tickets/T-0002/status", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ status: "open" }),
    });
    assert(res.ok, "done->open request succeeds");
    const after = fs.readFileSync(file, "utf8");
    assert(/^closed: $/m.test(after), "done->open: closed cleared");
    assert(checkVault(fakeVault).length === 0, "done->open: vault still lints clean");
  }

  // done -> archived keeps closed: it is the record of when the work finished.
  {
    const file = ticketFile(fakeVault, "T-0002"); // currently open, closed empty
    await fetch(BASE + "/api/tickets/T-0002/status", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ status: "done" }),
    });
    const today = localISODate(new Date());
    const res = await fetch(BASE + "/api/tickets/T-0002/status", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ status: "archived" }),
    });
    assert(res.ok, "done->archived request succeeds");
    const after = fs.readFileSync(file, "utf8");
    assert(after.includes(`closed: ${today}`), "done->archived: closed kept");
    assert(checkVault(fakeVault).length === 0, "done->archived: vault still lints clean");
  }

  // POST /log adds exactly one line. It changes nothing else.
  {
    const file = ticketFile(fakeVault, "T-0001");
    const before = fs.readFileSync(file, "utf8");
    const res = await fetch(BASE + "/api/tickets/T-0001/log", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ text: "test note from test.js" }),
    });
    assert(res.ok, "POST /log succeeds");
    const after = fs.readFileSync(file, "utf8");
    const { removed, added } = diffLines(before.split("\n"), after.split("\n"));
    assert(removed.length === 0 && added.length === 1, `POST /log: exactly one line added (removed=${removed.length}, added=${added.length})`);
    assert(/^- \d{4}-\d{2}-\d{2} \d{2}:\d{2} — test note from test\.js$/.test(added[0] || ""), "POST /log: line format correct");
    assert(checkVault(fakeVault).length === 0, "POST /log: vault still lints clean");
  }

  // Create: the id is max+1. The file name, the branch and the frontmatter match
  // the template.
  {
    const before = currentMaxId(fakeVault);
    const expectedId = "T-" + String(before + 1).padStart(4, "0");
    const res = await fetch(BASE + "/api/tickets", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ title: "New Ticket: Something", priority: "high", due: "2026-09-01" }),
    });
    const body = await res.json();
    assert(res.ok && body.id === expectedId, `create: id is max+1 (got ${JSON.stringify(body)}, want ${expectedId})`);
    assert(body.file === `tickets/${expectedId}-new-ticket-something.md`, `create: filename slug matches title (got ${body.file})`);
    const content = fs.readFileSync(path.join(fakeVault, body.file), "utf8");
    const fmKeys = content.match(/^---\n([\s\S]*?)\n---/)[1].split("\n").map((l) => l.split(":")[0]);
    const wantKeys = ["id", "title", "status", "priority", "project", "repos", "tags", "created", "due", "closed", "branch"];
    assert(JSON.stringify(fmKeys) === JSON.stringify(wantKeys), `create: frontmatter key order matches template (got ${JSON.stringify(fmKeys)})`);
    assert(content.includes('title: "New Ticket: Something"'), "create: title quoted correctly");
    assert(content.includes("status: open"), "create: status is open");
    assert(content.includes("priority: high"), "create: priority set from request");
    assert(content.includes("due: 2026-09-01"), "create: due set from request");
    assert(content.includes(`branch: ${expectedId}-new-ticket-something`), "create: branch matches T-NNNN-<slug>");
    assert(content.includes("## Summary\n\nNew Ticket: Something\n\n## Notes\n\n- \n\n## Log\n"), "create: body sections match template");
    assert(checkVault(fakeVault).length === 0, "create: vault still lints clean");
  }

  // Create with a title in Japanese only. The slug keeps no ASCII character.
  // Thus the file name and the branch use plain T-NNNN.
  {
    const before = currentMaxId(fakeVault);
    const expectedId = "T-" + String(before + 1).padStart(4, "0");
    const res = await fetch(BASE + "/api/tickets", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ title: "日本語のみのタイトル" }),
    });
    const body = await res.json();
    assert(res.ok && body.file === `tickets/${expectedId}.md`, `create (ja title): falls back to plain T-NNNN.md (got ${JSON.stringify(body)})`);
    const content = fs.readFileSync(path.join(fakeVault, body.file), "utf8");
    assert(content.includes(`branch: ${expectedId}\n`), "create (ja title): branch has no slug either");
    assert(checkVault(fakeVault).length === 0, "create (ja title): vault still lints clean");
  }

  // fields: a title change touches only the title line.
  {
    const file = ticketFile(fakeVault, "T-0004");
    const before = fs.readFileSync(file, "utf8");
    const res = await fetch(BASE + "/api/tickets/T-0004/fields", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ title: "Renamed title" }),
    });
    assert(res.ok, "fields: title update succeeds");
    const after = fs.readFileSync(file, "utf8");
    const { removed, added } = diffLines(before.split("\n"), after.split("\n"));
    assert(
      removed.length === 1 && added.length === 1 && added[0] === 'title: "Renamed title"',
      `fields: exactly the title line changed (removed=${JSON.stringify(removed)}, added=${JSON.stringify(added)})`
    );
    assert(checkVault(fakeVault).length === 0, "fields: vault still lints clean");
  }

  // fields: a title with `$&` must not trigger String.replace's special
  // replacement patterns, which would splice the old line into the new one.
  {
    const res = await fetch(BASE + "/api/tickets/T-0004/fields", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ title: "cost is $& real" }),
    });
    assert(res.ok, "fields: title with $& update succeeds");
    const content = await fetch(BASE + "/api/tickets/T-0004").then((r) => r.text());
    assert(content.includes('title: "cost is $& real"'), `fields: $& stored literally (got ${JSON.stringify(content.match(/^title:.*$/m)?.[0])})`);
    assert(checkVault(fakeVault).length === 0, "fields: vault still lints clean ($& title)");
  }

  // fields: a cleared due date uses the empty-field style of the vault again.
  {
    const file = ticketFile(fakeVault, "T-0001"); // the due date is 2026-08-14
    const res = await fetch(BASE + "/api/tickets/T-0001/fields", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ due: "" }),
    });
    assert(res.ok, "fields: due clear succeeds");
    const after = fs.readFileSync(file, "utf8");
    assert(/^due: $/m.test(after), "fields: due line back to empty style");
    assert(checkVault(fakeVault).length === 0, "fields: vault still lints clean (due clear)");
  }

  // GET /api/projects gives the top-level project names: projects/<name>.md
  // (flat) and projects/<name>/<name>.md (Obsidian folder note). A theme note
  // one level below a project folder, and a folder with no matching note, must
  // not show up.
  {
    const list = await fetch(BASE + "/api/projects").then((r) => r.json());
    assert(
      JSON.stringify(list) === JSON.stringify(["folder-note-demo", "vault-bootstrap"]),
      `GET /api/projects lists folder-note and flat projects only (got ${JSON.stringify(list)})`
    );
  }

  // Create: a folder-note project ("projects/<name>/<name>.md") is a valid
  // project. The frontmatter wikilinks its base name, same as a flat project.
  {
    const res = await fetch(BASE + "/api/tickets", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ title: "Folder note ticket", project: "folder-note-demo" }),
    });
    const body = await res.json();
    assert(res.ok, `create (folder-note project): request succeeds (got ${JSON.stringify(body)})`);
    const content = fs.readFileSync(path.join(fakeVault, body.file), "utf8");
    assert(content.includes('project: "[[folder-note-demo]]"'), "create (folder-note project): frontmatter wikilinks the base name");
    assert(checkVault(fakeVault).length === 0, "create (folder-note project): vault still lints clean");
  }

  // Create: a flat project ("projects/<name>.md") still works as before.
  {
    const res = await fetch(BASE + "/api/tickets", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ title: "Flat project ticket", project: "vault-bootstrap" }),
    });
    const body = await res.json();
    assert(res.ok, `create (flat project): request succeeds (got ${JSON.stringify(body)})`);
    const content = fs.readFileSync(path.join(fakeVault, body.file), "utf8");
    assert(content.includes('project: "[[vault-bootstrap]]"'), "create (flat project): frontmatter wikilinks the base name");
  }

  // Create: a theme note one level below a project folder is not itself a
  // project.
  {
    const res = await fetch(BASE + "/api/tickets", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ title: "Theme note ticket", project: "theme-note" }),
    });
    assert(res.status === 400, `create (theme-note as project): rejected with 400 (got ${res.status})`);
  }

  // Create: a project folder with no matching note is not a project.
  {
    const res = await fetch(BASE + "/api/tickets", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ title: "No-note-folder ticket", project: "no-note-folder" }),
    });
    assert(res.status === 400, `create (no-note-folder as project): rejected with 400 (got ${res.status})`);
  }

  // Create: a project name cannot escape projects/ via a path.
  {
    const res = await fetch(BASE + "/api/tickets", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ title: "Traversal ticket", project: "../../../etc/passwd" }),
    });
    assert(res.status === 400, `create (path traversal project): rejected with 400 (got ${res.status})`);
  }

  // fields: a project change touches only the project line.
  {
    const file = ticketFile(fakeVault, "T-0002"); // the project is empty
    const before = fs.readFileSync(file, "utf8");
    const res = await fetch(BASE + "/api/tickets/T-0002/fields", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ project: "vault-bootstrap" }),
    });
    assert(res.ok, "fields: project set succeeds");
    const after = fs.readFileSync(file, "utf8");
    const { removed, added } = diffLines(before.split("\n"), after.split("\n"));
    assert(
      removed.length === 1 && added.length === 1 && added[0] === 'project: "[[vault-bootstrap]]"',
      `fields: exactly the project line changed (removed=${JSON.stringify(removed)}, added=${JSON.stringify(added)})`
    );
    assert(checkVault(fakeVault).length === 0, "fields: vault still lints clean (project set)");

    // fields: a cleared project uses the empty-field style again.
    const res2 = await fetch(BASE + "/api/tickets/T-0002/fields", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ project: "" }),
    });
    assert(res2.ok, "fields: project clear succeeds");
    const after2 = fs.readFileSync(file, "utf8");
    assert(/^project: $/m.test(after2), "fields: project line back to empty style");
    assert(checkVault(fakeVault).length === 0, "fields: vault still lints clean (project clear)");
  }

  // fields: the server refuses an unknown project name. The file stays as it is.
  {
    const file = ticketFile(fakeVault, "T-0002");
    const before = fs.readFileSync(file, "utf8");
    const res = await fetch(BASE + "/api/tickets/T-0002/fields", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ project: "no-such-project" }),
    });
    assert(res.status === 400, `fields: unknown project rejected with 400 (got ${res.status})`);
    const after = fs.readFileSync(file, "utf8");
    assert(after === before, "fields: file untouched on unknown project");
  }

  // fields: the server refuses an unknown key. For example status, which has its
  // own endpoint.
  {
    const file = ticketFile(fakeVault, "T-0005");
    const before = fs.readFileSync(file, "utf8");
    const res = await fetch(BASE + "/api/tickets/T-0005/fields", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ status: "done" }),
    });
    assert(res.status === 400, `fields: unknown key rejected with 400 (got ${res.status})`);
    const after = fs.readFileSync(file, "utf8");
    assert(after === before, "fields: file untouched on rejected key");
  }

  // fields: set, replace and clear the tags block. Each write touches only the
  // tags block.
  {
    const file = ticketFile(fakeVault, "T-0901"); // made above with POST /api/tickets, tags: []
    const before0 = fs.readFileSync(file, "utf8");
    assert(/^tags: \[\]$/m.test(before0), "fields (tags): fixture starts with tags: []");

    // [] -> ["alpha", "beta"]
    let res = await fetch(BASE + "/api/tickets/T-0901/fields", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ tags: ["alpha", "beta"] }),
    });
    assert(res.ok, "fields (tags): [] -> list succeeds");
    let after = fs.readFileSync(file, "utf8");
    let d = diffLines(before0.split("\n"), after.split("\n"));
    assert(
      d.removed.length === 1 && d.removed[0] === "tags: []" &&
      JSON.stringify(d.added) === JSON.stringify(["tags:", "  - alpha", "  - beta"]),
      `fields (tags): [] -> list touches only the tags block (removed=${JSON.stringify(d.removed)}, added=${JSON.stringify(d.added)})`
    );
    assert(checkVault(fakeVault).length === 0, "fields (tags): vault still lints clean ([] -> list)");

    // ["alpha", "beta"] -> ["gamma"]
    const before1 = after;
    res = await fetch(BASE + "/api/tickets/T-0901/fields", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ tags: ["gamma"] }),
    });
    assert(res.ok, "fields (tags): list -> different list succeeds");
    after = fs.readFileSync(file, "utf8");
    d = diffLines(before1.split("\n"), after.split("\n"));
    // The "tags:" line has the same bytes before and after the write. Thus the
    // LCS diff reports no change for it. Only the item lines show.
    assert(
      JSON.stringify(d.removed) === JSON.stringify(["  - alpha", "  - beta"]) &&
      JSON.stringify(d.added) === JSON.stringify(["  - gamma"]),
      `fields (tags): list -> list touches only the tags bullets (removed=${JSON.stringify(d.removed)}, added=${JSON.stringify(d.added)})`
    );
    assert(checkVault(fakeVault).length === 0, "fields (tags): vault still lints clean (list -> list)");

    // ["gamma"] -> []
    const before2 = after;
    res = await fetch(BASE + "/api/tickets/T-0901/fields", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ tags: [] }),
    });
    assert(res.ok, "fields (tags): list -> [] succeeds");
    after = fs.readFileSync(file, "utf8");
    d = diffLines(before2.split("\n"), after.split("\n"));
    assert(
      JSON.stringify(d.removed) === JSON.stringify(["tags:", "  - gamma"]) &&
      JSON.stringify(d.added) === JSON.stringify(["tags: []"]),
      `fields (tags): list -> [] touches only the tags block (removed=${JSON.stringify(d.removed)}, added=${JSON.stringify(d.added)})`
    );
    assert(checkVault(fakeVault).length === 0, "fields (tags): vault still lints clean (list -> [])");
  }

  // fields: the server refuses an invalid tag with a space or a quote. The file
  // stays as it is.
  {
    const file = ticketFile(fakeVault, "T-0901");
    const before = fs.readFileSync(file, "utf8");
    const res = await fetch(BASE + "/api/tickets/T-0901/fields", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ tags: ["bad tag", 'quoted"tag'] }),
    });
    assert(res.status === 400, `fields (tags): invalid tag rejected with 400 (got ${res.status})`);
    const after = fs.readFileSync(file, "utf8");
    assert(after === before, "fields (tags): file untouched on invalid tag");
  }

  // sections: a write to Notes changes only the Notes section. The frontmatter,
  // the Summary section and the Log section keep their bytes.
  {
    const file = ticketFile(fakeVault, "T-0006");
    const before = fs.readFileSync(file, "utf8");
    const res = await fetch(BASE + "/api/tickets/T-0006/sections", {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ notes: "- rewritten note" }),
    });
    assert(res.ok, "sections: Notes replace succeeds");
    const after = fs.readFileSync(file, "utf8");
    const beforeUpToNotes = before.slice(0, before.indexOf("## Notes"));
    const afterUpToNotes = after.slice(0, after.indexOf("## Notes"));
    assert(beforeUpToNotes === afterUpToNotes, "sections: frontmatter + Summary byte-identical");
    assert(before.slice(before.indexOf("## Log")) === after.slice(after.indexOf("## Log")), "sections: Log section byte-identical");
    assert(after.includes("- rewritten note"), "sections: new Notes content present");
    assert(!after.includes(".claude/skills/ticket/SKILL.md"), "sections: old Notes content replaced, not appended");
    assert(checkVault(fakeVault).length === 0, "sections: vault still lints clean");
  }

  // sections: clear a section and fill it again, two times. The file must not
  // grow. Before the fix, a section with only whitespace made its empty lines
  // two times on each write.
  {
    const file = ticketFile(fakeVault, "T-0003");
    const putNotes = (notes) =>
      fetch(BASE + "/api/tickets/T-0003/sections", {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ notes }),
      });
    await putNotes("");
    const afterClear1 = fs.readFileSync(file, "utf8").split("\n").length;
    await putNotes("- refilled once");
    await putNotes("");
    const afterClear2 = fs.readFileSync(file, "utf8").split("\n").length;
    await putNotes("- refilled twice");
    await putNotes("");
    const afterClear3 = fs.readFileSync(file, "utf8").split("\n").length;
    assert(
      afterClear1 === afterClear2 && afterClear2 === afterClear3,
      `sections: clear/fill cycles don't grow the file (line counts ${afterClear1}, ${afterClear2}, ${afterClear3})`
    );
    assert(checkVault(fakeVault).length === 0, "sections: vault still lints clean after clear/fill cycles");
  }

  // sections: a ticket with CRLF line endings keeps CRLF after a section write.
  {
    const file = ticketFile(fakeVault, "T-0900"); // the CRLF fixture, see main()
    const res = await fetch(BASE + "/api/tickets/T-0900/sections", {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ notes: "- rewritten crlf note" }),
    });
    assert(res.ok, "sections (CRLF): request succeeds");
    const after = fs.readFileSync(file, "utf8");
    assert(!/(?<!\r)\n/.test(after), "sections (CRLF): every newline still preceded by \\r");
    assert(after.includes("- rewritten crlf note\r"), "sections (CRLF): new content uses CRLF too");
    assert(!after.includes("original crlf note"), "sections (CRLF): old note replaced, not appended");
    assert(checkVault(fakeVault).length === 0, "sections (CRLF): vault still lints clean");
  }

  // The server refuses an invalid status. The file keeps every byte.
  {
    const file = ticketFile(fakeVault, "T-0005");
    const before = fs.readFileSync(file, "utf8");
    const res = await fetch(BASE + "/api/tickets/T-0005/status", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ status: "bogus" }),
    });
    assert(res.status === 400 || res.status === 422, `invalid status rejected with 4xx (got ${res.status})`);
    const after = fs.readFileSync(file, "utf8");
    assert(after === before, "invalid status: file untouched");
  }

  // The server refuses a path traversal in :id. The slash is encoded. Thus it
  // stays in the path after URL normalization.
  {
    const res = await fetch(BASE + "/api/tickets/" + encodeURIComponent("../../etc/passwd"));
    assert(res.status === 400, `path traversal id rejected with 400 (got ${res.status})`);
  }

  // Duplicate frontmatter keys must be rejected. parseFrontmatter reads
  // first-wins (to match the TUI), so a duplicate key is a real ambiguity
  // between "what the file means" and "what a naive second read would give
  // you" — not something to resolve quietly.
  {
    const errors = lintBrokenVault((tmp) => {
      const file = ticketFile(tmp, "T-0002"); // status: open
      const text = fs.readFileSync(file, "utf8");
      fs.writeFileSync(file, text.replace("status: open\n", "status: open\nstatus: doing\n"), "utf8");
    });
    assert(
      errors.some((e) => /duplicate frontmatter key "status"/.test(e)),
      `lint rejects a duplicate frontmatter key (got ${JSON.stringify(errors)})`
    );
  }

  // A frontmatter terminator that isn't exactly "---" on its own line must be
  // rejected, e.g. "---oops" — the TUI requires an exact match.
  {
    const errors = lintBrokenVault((tmp) => {
      const file = ticketFile(tmp, "T-0002");
      const lines = fs.readFileSync(file, "utf8").split("\n");
      const closeIdx = lines.indexOf("---", 1); // the closing "---", not the opening one
      lines[closeIdx] = "---oops";
      fs.writeFileSync(file, lines.join("\n"), "utf8");
    });
    assert(
      errors.some((e) => /missing frontmatter/.test(e)),
      `lint rejects a "---oops" frontmatter terminator (got ${JSON.stringify(errors)})`
    );
  }

  // A title with an escape outside the supported table (\\ \" \/ \n \t \r \0
  // \uXXXX \xXX) must be rejected, not silently corrupted.
  {
    const errors = lintBrokenVault((tmp) => {
      const file = ticketFile(tmp, "T-0002");
      const text = fs.readFileSync(file, "utf8");
      fs.writeFileSync(file, text.replace(/^title: .*$/m, 'title: "bad\\q"'), "utf8");
    });
    assert(
      errors.some((e) => /unknown escape in title/.test(e)),
      `lint rejects an unknown escape in title (got ${JSON.stringify(errors)})`
    );
  }

  // A lone surrogate escape (\uD800) must be rejected too: Rust's
  // char::from_u32 refuses a surrogate scalar, so unescapeYaml/hasUnknownEscape
  // treat the whole \uD800-\uDFFF range as an unknown escape in both front
  // ends. Without this, Node would decode it (as a UTF-16 code unit) while
  // Rust would not, and the lint would let the mismatch through.
  {
    const errors = lintBrokenVault((tmp) => {
      const file = ticketFile(tmp, "T-0002");
      const text = fs.readFileSync(file, "utf8");
      fs.writeFileSync(file, text.replace(/^title: .*$/m, 'title: "bad\\uD800"'), "utf8");
    });
    assert(
      errors.some((e) => /unknown escape in title/.test(e)),
      `lint rejects a lone surrogate \\uXXXX escape in title (got ${JSON.stringify(errors)})`
    );
  }

  // A surrogate-pair escape (😀, the two UTF-16 halves of U+1F600
  // "grinning face") is rejected the same way: Rust does not compose
  // surrogate pairs from two \uXXXX escapes, so neither half is known.
  {
    const errors = lintBrokenVault((tmp) => {
      const file = ticketFile(tmp, "T-0002");
      const text = fs.readFileSync(file, "utf8");
      fs.writeFileSync(file, text.replace(/^title: .*$/m, 'title: "bad\\uD83D\\uDE00"'), "utf8");
    });
    assert(
      errors.some((e) => /unknown escape in title/.test(e)),
      `lint rejects a surrogate-pair \\uXXXX escape in title (got ${JSON.stringify(errors)})`
    );
  }

  // closed set while status is open contradicts the schema — closed may only
  // be set once status reaches done or archived. This is the mirror of the
  // existing "done but closed empty" check.
  {
    const errors = lintBrokenVault((tmp) => {
      const file = ticketFile(tmp, "T-0002"); // status: open, closed: (empty)
      const text = fs.readFileSync(file, "utf8");
      fs.writeFileSync(file, text.replace("closed: \n", "closed: 2026-01-01\n"), "utf8");
    });
    assert(
      errors.some((e) => /closed is set but status is "open"/.test(e)),
      `lint rejects closed set while status is open (got ${JSON.stringify(errors)})`
    );
  }

  // unquote() decodes the YAML double-quoted escapes this vault needs: a
  // \uXXXX code point, a literal backslash, and a tab. Written straight into
  // the fakeVault (not the committed fixture) since these are valid,
  // lint-clean tickets.
  {
    const escapeFixture = (id, rawTitle) =>
      [
        "---",
        `id: ${id}`,
        `title: "${rawTitle}"`,
        "status: open",
        "priority: normal",
        "project: ",
        "repos: []",
        "tags: []",
        "created: 2026-08-09",
        "due: ",
        "closed: ",
        "branch: ",
        "---",
        "",
        "## Summary",
        "",
        "Fixture for verifying YAML escape decoding.",
        "",
        "## Notes",
        "",
        "- ",
        "",
        "## Log",
        "",
        "- 2026-08-09 00:00 — created",
        "",
      ].join("\n");
    fs.writeFileSync(path.join(fakeVault, "tickets", "T-0910-escape-unicode.md"), escapeFixture("T-0910", "Smile \\u263A"), "utf8");
    fs.writeFileSync(path.join(fakeVault, "tickets", "T-0911-escape-backslash.md"), escapeFixture("T-0911", "a\\\\b"), "utf8");
    fs.writeFileSync(path.join(fakeVault, "tickets", "T-0912-escape-tab.md"), escapeFixture("T-0912", "tab\\there"), "utf8");
    // An astral character (above U+FFFF) written literally, not as a \uXXXX
    // escape, is not affected by the surrogate-escape rejection above — both
    // front ends read raw UTF-8 the same way.
    fs.writeFileSync(path.join(fakeVault, "tickets", "T-0913-escape-literal-astral.md"), escapeFixture("T-0913", "Smile 😀"), "utf8");
    assert(checkVault(fakeVault).length === 0, "escape fixtures: vault still lints clean");
    const list = await fetch(BASE + "/api/tickets").then((r) => r.json());
    const byId = (id) => list.find((t) => t.id === id);
    assert(byId("T-0910")?.title === "Smile ☺", `\\uXXXX decodes to the code point (got ${JSON.stringify(byId("T-0910")?.title)})`);
    assert(byId("T-0911")?.title === "a\\b", `\\\\ decodes to one backslash (got ${JSON.stringify(byId("T-0911")?.title)})`);
    assert(byId("T-0912")?.title === "tab\there", `\\t decodes to a tab (got ${JSON.stringify(byId("T-0912")?.title)})`);
    assert(byId("T-0913")?.title === "Smile 😀", `a literal astral character round-trips as-is (got ${JSON.stringify(byId("T-0913")?.title)})`);
  }

  // A folder note project only counts if projects/<name>/<name>.md is a file.
  // A same-named directory (an Obsidian artifact, or just a mistake) must not
  // turn a folder into a project.
  {
    fs.mkdirSync(path.join(fakeVault, "projects", "ghost", "ghost.md"), { recursive: true });
    const list = await fetch(BASE + "/api/projects").then((r) => r.json());
    assert(!list.includes("ghost"), `GET /api/projects excludes a folder note that is itself a directory (got ${JSON.stringify(list)})`);
  }

  // writeTicket's rollback has two branches: restore original content for an
  // existing file (covered below), and unlink for a file that did not exist
  // before the write — only exercised by ticket *creation*. Trip checkVault
  // via the same EISDIR trick, but nested one level down so checkVault's
  // recursive scan finds it while nextTicketId's plain, non-recursive
  // tickets/ scan does not — nextTicketId must succeed so the run actually
  // reaches writeTicket.
  {
    const trapDir = path.join(fakeVault, "tickets", "trap-subdir", "trap.md");
    fs.mkdirSync(trapDir, { recursive: true });
    const before = currentMaxId(fakeVault);
    const expectedId = "T-" + String(before + 1).padStart(4, "0");
    const expectedFile = path.join(fakeVault, "tickets", `${expectedId}-rollback-check.md`);
    const res = await fetch(BASE + "/api/tickets", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ title: "rollback-check" }),
    });
    assert(res.status === 500, `create: checkVault exception surfaces as 500 (got ${res.status})`);
    fs.rmSync(path.join(fakeVault, "tickets", "trap-subdir"), { recursive: true, force: true });
    assert(!fs.existsSync(expectedFile), "create: writeTicket deletes the half-written file when checkVault() throws");
    assert(checkVault(fakeVault).length === 0, "create rollback: vault lints clean again once the trap is removed");
  }

  // If checkVault() throws instead of returning problems (e.g. a stray
  // directory under tickets/ that looks like a ticket file), writeTicket must
  // still restore the original content before the error propagates. This is
  // the last test: it leaves fakeVault permanently broken for checkVault, on
  // purpose, by design of the reproduction below.
  {
    fs.mkdirSync(path.join(fakeVault, "tickets", "broken.md")); // makes checkVault's readFileSync EISDIR
    const file = ticketFile(fakeVault, "T-0001");
    const before = fs.readFileSync(file, "utf8");
    const res = await fetch(BASE + "/api/tickets/T-0001/log", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ text: "should be rolled back" }),
    });
    assert(res.status === 500, `writeTicket surfaces the checkVault exception (got ${res.status})`);
    const after = fs.readFileSync(file, "utf8");
    assert(after === before, "writeTicket restores the original file when checkVault() throws");
  }
}

async function main() {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "usher-test-"));
  const fakeVault = path.join(tmp, "vault");
  copyVaultInto(fakeVault);
  const checkVault = require(path.join(fakeVault, "system", "scripts", "check_vault.js"));

  // A fixture with CRLF line endings. It shows if a section write keeps the line
  // endings.
  const crlfContent = [
    "---",
    "id: T-0900",
    'title: "CRLF fixture"',
    "status: open",
    "priority: normal",
    "project: ",
    "repos: []",
    "tags: []",
    "created: 2026-08-09",
    "due: ",
    "closed: ",
    "branch: ",
    "---",
    "",
    "## Summary",
    "",
    "Fixture for verifying CRLF preservation.",
    "",
    "## Notes",
    "",
    "- original crlf note",
    "",
    "## Log",
    "",
    "- 2026-08-09 00:00 — created",
    "",
  ].join("\r\n");
  fs.writeFileSync(path.join(fakeVault, "tickets", "T-0900-crlf-fixture.md"), crlfContent, "utf8");

  const server = spawn(process.execPath, [path.join(__dirname, "server.js"), fakeVault], {
    env: { ...process.env, PORT: "0" }, // a free port from the OS, never the port of another process
    stdio: ["ignore", "pipe", "pipe"],
  });
  server.stderr.on("data", (d) => process.stderr.write(d));

  try {
    BASE = `http://localhost:${await waitForPort(server)}`;
    await runTests(fakeVault, checkVault);
  } finally {
    const exited = new Promise((resolve) => server.once("exit", resolve));
    server.kill();
    await exited; // an early exit() can cause a libuv handle-close race on Windows
    fs.rmSync(tmp, { recursive: true, force: true });
  }

  console.log(process.exitCode ? "\nFAIL" : "\nALL PASS");
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
