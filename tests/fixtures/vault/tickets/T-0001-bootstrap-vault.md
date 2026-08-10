---
id: T-0001
title: "Bootstrap the vault"
status: doing
priority: high
project: "[[vault-bootstrap]]"
repos:
  - "[[example-repo]]"
tags:
  - setup
created: 2026-08-09
due: 2026-08-14
closed: 
branch: T-0001-bootstrap-vault
---

## Summary

Set up this freshly cloned vault. This ticket doubles as a living example of the schema — open its properties to see every field in use.

## Notes

- [ ] Run `node system/scripts/install_plugins.js` from the vault root (downloads pinned Templater / Calendar Bases / Kanban Bases View), then enable community plugins (trust dialog / turn off Restricted mode) — settings are pre-wired
- [ ] Register real repositories in `repos/` (both `path_windows` and `path_wsl`)
- [ ] Point git remote at your own repository, make the initial commit
- [ ] Create your first real ticket via Templater → `ticket`
- [ ] Delete `repos/example-repo.md`, then close this ticket (`status: done`, set `closed`)

## Log

- 2026-08-09 10:00 — created
