# Performance budget

Budget (spec §1.2), measured with `scripts/measure-footprint.ps1` on a release build.
Memory = private working set (Task Manager "Memory" column); CPU = share of all logical processors.

| Milestone | Machine | Mode | App CPU % | App private MB | WebView2 procs | Total private MB | Budget met |
|---|---|---|---|---|---|---|---|
| M1 | AMD Ryzen 7 7800X3D, 32 GB RAM, Windows 11 Pro 10.0.26200 | window | 0 | 15.0 | 6 | 112.5 | yes |
| M1 | same machine | tray | 0.01 | 12.3 | 0 | 12.3 | yes |

Budget: app CPU < 1 % at idle; tray < 30 MB; window open < 200 MB in total.
