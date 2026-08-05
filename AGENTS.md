# Project instructions

## Repository layout

- Keep all Rust and Cargo files inside `firmware/`.
- Keep the standalone WebBLE pages inside `dashboard/`.
- Keep KiCad source files and committed fabrication artifacts inside `hardware/`.

## Firmware

- Run firmware commands from `firmware/`.
- Use debug builds by default.
- Do not use `--release` or otherwise produce release builds unless the user explicitly requests one.
- Flash the firmware with `cargo embed` without `--release` by default.

## Dashboard

- The dashboards are standalone HTML files with inline CSS and JavaScript.
- Do not add React, Node modules, CDNs, or runtime dependencies.
- WebBLE testing should use localhost or another browser-approved secure context.

## Hardware

- Keep KiCad source files, the schematic PDF, and the fabrication ZIP under `hardware/`.
- Do not commit KiCad backups, editor state, caches, raw fabrication outputs, or drill files.
