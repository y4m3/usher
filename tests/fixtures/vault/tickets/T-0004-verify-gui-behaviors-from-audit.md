---
id: T-0004
title: "Verify GUI behaviors flagged by the audit"
status: open
priority: normal
project: "[[vault-bootstrap]]"
repos: []
tags:
  - maintenance
created: 2026-08-09
due: 
closed: 
branch: T-0004-verify-gui-behaviors-from-audit
---

## Summary

The robustness audit ([[T-0003]]) left four behaviors that can only be
confirmed in the running Obsidian app. Check each one in the real vault (or
the dev vault) and log the outcome here.

## Notes

- [ ] Board view: drag a card between columns → `status` property updates
- [ ] Calendar view: drag a ticket to another day → `due` property updates
- [ ] `Alt+N` opens the plain-note template (Templater's own default binding
      was cleared in `hotkeys.json` — confirm no fuzzy-search popup appears)
- [ ] Bases `file.inFolder("tickets")`: does a ticket parked in a subfolder
      (e.g. `tickets/archive/`) still appear in the views? If not, decide
      whether the lint should forbid subfolders

## Log

- 2026-08-09 07:07 — created
