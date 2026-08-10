---
id: T-0006
title: "Ship Claude Code skills with the template"
status: done
priority: normal
project: "[[vault-bootstrap]]"
repos: []
tags:
  - launch
created: 2026-08-09
due: 
closed: 2026-08-09
branch: T-0006-ship-claude-code-skills
---

## Summary

Bundle Claude Code skills so template users get `/ticket` and
`/weekly-review` out of the box.

## Notes

- `.claude/skills/ticket/SKILL.md` — create a ticket per AGENTS.md
- `.claude/skills/weekly-review/SKILL.md` — run the weekly review
- Both are thin wrappers over AGENTS.md (no duplicated schema) and
  self-contained: they resolve the vault from the current directory, then
  the `TICKET_VAULT` environment variable, then by asking — so they keep
  working when copied into the WSL user scope (`~/.claude/skills/`) while
  the vault lives on the Windows side
- README "Working with agents" documents the copy/symlink setup and
  `TICKET_VAULT`

## Log

- 2026-08-09 08:35 — created
- 2026-08-09 08:35 — done: skills added, README updated, lint clean
