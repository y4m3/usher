---
id: T-0002
title: "Unpin Templater once the 2.25.x load regression is fixed"
status: open
priority: low
project: 
repos: []
tags:
  - maintenance
created: 2026-08-09
due: 
closed: 
branch: 
---

## Summary

Templater 2.25.0 (released 2026-08-05) fails to load on Obsidian 1.13.x, so `system/scripts/install_plugins.js` installs a pinned **2.24.3** and the README warns against updating. This is a temporary measure — lift it when upstream ships a fix.

## Notes

- Watch [silentvoid13/Templater releases](https://github.com/silentvoid13/Templater/releases) for a version > 2.25.0
- When fixed: bump the `version` in `system/scripts/install_plugins.js`, re-run it, retest ticket creation + daily notes, and remove the pin warning from `README.md`

## Log

- 2026-08-09 04:45 — created
- 2026-08-09 05:30 — Templater is no longer bundled (AGPL caution for the public repo); the pin now lives in `install_plugins.js`
