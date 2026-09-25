# OpenMonitor Advanced

Open-source hardware monitor for Windows with a modern UI: a Simple view that tells you at a glance
whether your PC is fine, and an Advanced view with every sensor.

**Status:** milestone 1 (foundations) — CPU, RAM, disks and network without admin rights.
Design: `docs/superpowers/specs/2026-09-24-openmonitor-advanced-design.md`.

## Build

Prerequisites: Windows 10/11, Rust stable ≥ 1.85 (MSVC), Node 22, pnpm 10, WebView2 runtime (preinstalled on Windows 11).

    cd app
    pnpm install
    pnpm tauri dev        # run the Tauri shell against the built UI
    pnpm dev              # UI only, in the browser, with a mock backend, with hot reload

Tests: `cargo test --workspace` (after `pnpm build` in `app/`) and `pnpm test` in `app/`.
Before closing a milestone, also run the hardware tests on real Windows hardware:
`cargo test -p oma-win -- --include-ignored`.

## Performance budget

The monitor must not distort what it measures. Budgets and the latest measurements are in
`docs/perf-budget.md`; run `scripts/measure-footprint.ps1` on a release build to reproduce them.

## License

GPL-3.0-or-later. See `LICENSE`.
