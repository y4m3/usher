---
id: T-0003
title: "Fix the robustness-audit findings"
status: done
priority: high
project: "[[vault-bootstrap]]"
repos: []
tags:
  - maintenance
created: 2026-08-09
due: 
closed: 2026-08-09
branch: T-0003-fix-robustness-audit-findings
---

## Summary

A 15-subagent audit (Haiku/Sonnet scenario runs, 16-error corruption battery,
recovery test, fresh-clone walkthrough) confirmed the vault survives cheap
models and dumb operations, but surfaced 8 fixable findings. This ticket
applies all of them.

## Notes

- `check_vault.js`: accept quoted YAML scalars in every field (was a false
  positive), report `missing <field>` instead of leaking `"undefined"`, and
  fail with a clear message when run outside a vault root
- `AGENTS.md`: PowerShell ID-numbering one-liner; `T-9999` cap noted;
  concurrent-creation collision resolution; branch is set at creation
  (aligned with the ticket template); external facts must be verified against
  the primary source before closing (else stop at `review`); weekly-review
  output goes to the conversation unless asked; never delete ticket files —
  archive instead
- `README.md`: lint surfaced for humans in Daily use; *Due radar* wording now
  names the actual *Due soon* view
- `.obsidian/hotkeys.json`: cleared Templater's default `Alt+N` binding
  (`create-new-note-from-template`) which shadowed the vault's plain-note
  hotkey — GUI verification tracked in [[T-0004]]

## Log

- 2026-08-09 07:07 — created
- 2026-08-09 07:07 — started; audit report and evidence live in the session
  sandboxes (throwaway clones), real vault untouched during the audit
- 2026-08-09 07:08 — done: applied all 8 fixes across check_vault.js,
  AGENTS.md, README.md, hotkeys.json; lint re-run clean. No commits — the
  vault owner controls git
