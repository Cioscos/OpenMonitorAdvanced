# Ranking ("classifica") for OpenMonitor Advanced benchmarks: research report

Date of research: 2026-10-06. Items marked **[unverified]** come from general knowledge and could not be confirmed by a fetched page; free-tier numbers of third parties change often and must be re-checked before committing.

## 0. Short answer

- Every established benchmark uses one of two models: (1) **crowd-sourced aggregate** (Geekbench Browser, PassMark, UserBenchmark, Blender Open Data, OpenBenchmarking, CPU-Z Validator, 3DMark), or (2) **reference table shipped inside the app** (CPU-Z Bench dropdown of reference CPUs; Cinebench's built-in list [unverified]). Model 2 needs no backend at all.
- Anti-cheat for an open-source client is not achievable. Every crowd-sourced system lives with it through aggregation (median/mean per hardware model, minimum samples, filters), not prevention. Blender Open Data, the closest analogue, has open issues about tampered `.blend` files and absurd Metal values.
- An online leaderboard is not "too complicated" in the technical sense (a Cloudflare Worker + D1 fits in the free tier for years), but the **recurring cost is moderation, privacy/GDPR duties and score-version management**, for a one-person project. Recommended path: ship a **local ranking against a bundled reference table + user's own history first**, design the score format so that it can be submitted later, and add an **opt-in, GitHub-/Worker-based submission** only after the reference table proves its worth.

## 1. How established benchmarks build rankings

### 1.1 Geekbench (Primate Labs)
- Scores are **calibrated to a baseline machine**. Geekbench 5: baseline 1000 = Intel Core i3-8100. Geekbench 6: baseline 2500 = Intel Core i7-12700 (search-result summary of Primate's docs; the PDF "Geekbench 6 Benchmark Internals" could not be parsed by the fetch tool). Source: https://www.geekbench.com/doc/geekbench6-benchmark-internals.pdf
- Composite score = **weighted arithmetic mean of section scores; each section score = geometric mean of its workload scores** (same source as above, via search summary). Geometric means avoid one workload dominating, and make "ratio to reference" the natural unit (double score = double speed).
- Free version **always uploads** results publicly; paid license can run offline (https://apps.apple.com/us/app/-/id1565728895 and search results). Processor chart: "only includes processors with at least five unique results" (https://browser.geekbench.com/processor-benchmarks, quoted in search results; direct fetch returned 403).
- Score **versioning**: each major version (5, 6, 7) has its own scale, own browser section and its own baseline; results are never mixed across versions (Geekbench 7 exists: https://signal65.com/research/geekbench-7-analysis-and-early-results/).
- Pre-release hardware policy exists to limit leak/abuse: https://primatelabs.com/policy/prerelease-hardware.html

### 1.2 PassMark (cpubenchmark.net, PerformanceTest)
- Data: "gathered from users' submissions to the PassMark web site as well as from internal testing" (https://www.cpubenchmark.net/cpu_test_info.html).
- Aggregation: **average of all submissions for a given CPU model**; main charts exclude overclocked submissions; invalid/duplicate-from-same-system submissions are filtered "automatically and manually, as time allows"; minimum two samples for a chart entry; charts recalculated daily (https://forums.passmark.com/pc-hardware-and-benchmarks/48456-how-are-average-cpu-benchmarks-calculated , https://forums.passmark.com/pc-hardware-and-benchmarks/4974-of-samples). No published outlier algorithm.
- Versioning: PerformanceTest 9 vs 10 results are explicitly not comparable because the CPU tests changed (cpu_test_info page).

### 1.3 UserBenchmark and its criticism
- User-submitted, averaged per model, weighted "effective speed" index. In July 2019 the weighting became roughly 40% single-core / 58% quad-core / 2% multi-core, so a Core i3-8100 ranked above an i9-9980XE; community accused it of anti-AMD bias; r/hardware banned links in 2020; later a paid Pro tier (https://en.wikipedia.org/wiki/UserBenchmark , https://www.tomshardware.com/news/userbenchmark-benchmark-change-criticism-amd-intel%2C40032.html).
- Lessons: (a) **never collapse into one opaque composite with arbitrary weights**; show single and multi separately and publish the formula; (b) editorial content next to a ranking destroys trust; (c) changing weights retroactively reshuffles every ranking.

### 1.4 3DMark / UL Hall of Fame
- Hall of Fame = **top-100 only**, by overclockers; UL validates results; requires latest benchmark + SystemInfo version, approved WHQL drivers, public hardware, default settings/presets; "provided for entertainment" (https://benchmarks.ul.com/hall-of-fame-2/ ...). This is a validation-by-gatekeeping model (a company with staff), not applicable to us.
- Typical per-score comparison text ("faster than X% of all results") [unverified, from general knowledge].

### 1.5 Cinebench (Maxon)
- Cinebench R15/R20/R23 had an in-app ranking/reference list so the result could be compared offline, I recall this from the app UI [unverified; no source found]. Cinebench 2024 adds GPU (Redshift) and is mainly compared through third-party tables (https://nanoreview.net/en/cpu-list/cinebench-scores). Relevant idea: **the reference table is shipped with the app and the user's result is inserted in it**.

### 1.5b CPU-Z (CPUID)
- CPU-Z Bench: run single/multi thread, then compare against a **drop-down list of reference CPUs** embedded in the app; optional submission to the CPU-Z Validator site where a graph shows where the CPU ranks among thousands of submissions (https://valid.cpuid.com/bench/20, https://www.techradar.com/how-to/how-to-use-cpu-z). Bench versions are explicit ("x64 - 2017.1") and rankings are per version.

### 1.6 CrystalDiskMark
- No ranking, no upload. Local numbers only. (General knowledge; confirms "disk results with no ranking" is acceptable for users.) Disk results are the hardest to rank fairly anyway: depend on queue depth, file size, free space, filesystem, drive fullness, thermal state, and the drive's SLC cache.

### 1.7 Blender Open Data (closest analogue)
- Open-source (code at https://projects.blender.org/infrastructure/blender-open-data), run by the Blender Foundation. Four parts: website, launcher, websocket authenticator (token-based), benchmark script (https://projects.blender.org/Jeff-Allen/blender-open-data).
- User runs benchmark, then **chooses to share online or save locally**; system info (OS, RAM, GPUs, CPU model) is gathered; data is anonymous by default, optional display name, no PII (https://opendata.blender.org/about/ ; https://www.blender.org/news/introducing-blender-benchmark/ ). Results are **public domain / CC0**, downloadable as JSON and CSV, with daily snapshots, no registration required to read.
- Score = estimated Cycles samples per minute summed over scenes; results are **grouped per Blender version** (page defaults to 5.2.0) and per device type (OPTIX/CUDA/HIP/METAL/ONEAPI). 344,226 benchmarks shown in total at the time of writing (https://opendata.blender.org/).
- Aggregation is **median per device name + type**. Known problems from their tracker: users can edit the local `.blend` files before benchmarking (https://projects.blender.org/archive/blender-benchmark-bundle/issues/76290), absurd Metal values (https://projects.blender.org/archive/blender-benchmark-bundle/issues/96519), proposal to use the 10th percentile instead of median and to limit homepage data to the last year (https://projects.blender.org/infrastructure/blender-open-data/issues/73322). So even a big foundation with a client token flow has no real anti-forgery; the median per model just makes forged outliers irrelevant when sample counts are healthy.

### 1.8 Phoronix OpenBenchmarking.org
- Open-source client (Phoronix Test Suite), **opt-in upload**; the server provides a **percentile ranking** of your result against comparable public results, and the client queries an API for "interesting" comparable results for real-time comparison; statistics only where "sufficient statistically significant data" exists (https://www.phoronix.com/news/OB-Seamless-Comparisons-RFC , https://mail.openbenchmarking.org). Test profiles are versioned (a result is only comparable within the same test version).

### 1.9 Cross-cutting design principles
1. **Normalize to a reference machine** (Geekbench: baseline = 2500 / 1000). Score = 1000 * (our time on reference / our time on this machine) per workload, aggregated with a **geometric mean**. Lets you add or retire workloads without breaking the scale, and makes an "x times faster than reference" statement possible.
2. **Version everything**: `score_version` (workload + normalization). Never merge across versions. Keep old tables when possible, or mark them "legacy". Blender, Geekbench, PassMark, CPU-Z all do this.
3. **Per-model aggregation with a minimum sample count** (Geekbench: 5; PassMark: 2) and **median or filtered mean**, not top-100 only.
4. **Exclude abnormal runs** locally: thermal throttling flags, power-saver plan, battery, overclock/boost anomalies, run with too much other CPU load (the app is a hardware monitor, so it already knows background load and temperatures: a real advantage for "validity flags").
5. Publish the **formula and workloads** (UserBenchmark lesson).

## 2. Options for an open-source project with no backend

Common data model needed by any option (see 2.5): `score_version`, workload scores, hardware model strings, minimal environment.

### 2a. Local-only: bundled reference table + user's own history
- **What**: ship `reference-scores.json` inside the app (hardware model -> single, multi, per-workload scores, score_version), compare the user's result against it, show percentile/bar position, and store local history (already compatible with the app's CSV/history modules).
- **How to get reference numbers legally**
  - Measure ourselves (the author's machines: RTX 4080 + AMD iGPU; borrowed machines, VMs/cloud instances for CPUs). Facts/measurements produced by us are free of third-party rights.
  - **Community PRs**: contributors run the benchmark and open a PR/issue with the exported JSON (the app has an "export result" button). Licence the dataset CC0 like Blender, stated in `CONTRIBUTING`/README so contributors agree by submitting.
  - Do **not** copy tables from PassMark, Geekbench, UserBenchmark, Cinebench sites: the numbers are on a different scale anyway and do not apply to our workloads; also the EU sui generis database right and website ToS can bar bulk extraction (https://www.jacobacci-law.com/news-and-events/cjeu-c-762-19-the-sui-generis-right-of-the-database-maker , summary of the general principle only; legal advice not obtained). Cross-conversion from other benchmarks (e.g. "scale to Cinebench") is technically possible but dubious and pointless.
  - A smart bootstrap: since scores are normalized to a baseline machine, the table can just contain a **few dozen anchor models** (low end iGPU, mid, high end, typical laptops) and show "between X and Y" rather than a fine ranking.
- **Effort**: low (1 JSON schema + UI list + percentile function; days). Update cadence = app release.
- **Cost**: 0. **Abuse risk**: none (nothing is accepted from users at runtime). **GDPR**: nothing leaves the PC, so no controller duties at all.
- **Weaknesses**: table ages; coverage of the user's exact model is partial (show nearest tier, never fake precision); the quality depends on contributors; no "global percentile".

### 2b. GitHub-based submissions (no server of our own)
- **Flow**: app shows a "Share this result" button that opens `https://github.com/Cioscos/OpenMonitorAdvanced/issues/new?template=...&title=...&body=...` pre-filled (query params `title`, `body`, `template`, `labels` are supported, https://docs.github.com/en/issues/tracking-your-work-with-issues/using-issues/creating-an-issue#creating-an-issue-from-a-url-query). A GitHub Action validates the JSON (schema, plausibility vs reference) and either auto-commits to `data/submissions/*.json` or lets the maintainer approve by label; a scheduled Action aggregates (median per model per `score_version`) into `data/aggregate-vN.json`, published on GitHub Pages and/or fetched by the app. Pattern is well established (issue-form -> validate -> commit -> publish).
- **Limits (facts)**
  - Prefilled URL: practical cap about 8192 characters; issue body up to 65,536 chars. A compact submission (< 1 KB JSON) fits easily (https://claudeissues.com/issue/6895-bug-github-issue-url-exceeds-maximum-length-limit , from developer reports).
  - Raw file / REST access: unauthenticated limit is **60 requests/hour per IP** on the REST API (https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api); GitHub in May 2025 tightened unauthenticated limits for REST API, clones and **raw.githubusercontent.com** downloads (https://github.blog/changelog/2025-05-08-updated-rate-limits-for-unauthenticated-requests/), and people hit 429s on raw (https://github.com/morpheus65535/bazarr/issues/3057). Exact raw thresholds are not published. **Mitigation**: serve the aggregate via **GitHub Pages** (CDN, soft 100 GB/month, 1 GB site size, 10 builds/hour soft limit that does not apply to custom Actions deploys; no commercial SaaS use; may return 429; https://docs.github.com/en/pages/getting-started-with-github-pages/github-pages-limits) or as a **release asset**, and cache with ETag and at most one fetch per day per client. A single aggregate file of tens of KB is negligible.
- **Anti-abuse**: submitter needs a GitHub account (a mild barrier and a natural per-user rate limit), maintainer approval or automated plausibility checks, can block users, can revert via git history (full audit trail).
- **Privacy**: everything is **public and permanent** (git history). The submitter's GitHub username is attached to the issue; hardware strings go public. Needs clear "this will be published under CC0" text in the pre-filled issue body and an explicit user click. Deleting later means rewriting data and issues (an erasure request is feasible but manual). Never include serial numbers, GUIDs, hostnames, or IPs.
- **Effort**: medium-low (issue form + schema + 2 Actions + a fetch in the app; ~1 week). **Cost**: 0. **Scale ceiling**: tens to few hundreds of submissions/day manageable; at thousands, git-as-database gets silly (repo bloat, Action minutes), but a bot can batch-commit and prune to aggregates only.
- **UX friction**: requires a GitHub account; expect low participation (single digits to dozens of submissions per month at the beginning). Fine for a niche hardware monitor, but the "global percentile" would be statistically thin except for popular models.

### 2c. Serverless backend
Free-tier numbers verified on official pages unless noted (re-check at decision time).

| Option | Free allowance | Limits that bite | Paid |
|---|---|---|---|
| **Cloudflare Workers + D1** | Workers Free 100,000 requests/day, 10 ms CPU/request, 50 subrequests (https://developers.cloudflare.com/workers/platform/limits/). D1 free: 5 M rows read/day, 100,000 rows written/day, 5 GB account storage, 500 MB/db, 10 databases (https://developers.cloudflare.com/d1/platform/pricing/ , https://developers.cloudflare.com/d1/platform/limits/) | When daily D1 limits are hit, queries fail until 00:00 UTC (no surprise bill; this is a safe failure mode for hobby use) | Workers Paid $5/month min, 10 M requests included; D1 beyond free: $0.001 per M rows read, $1 per M rows written (https://developers.cloudflare.com/workers/platform/pricing/) |
| **Cloudflare Turnstile** (anti-bot) | Free, unlimited challenges, 20 widgets (https://developers.cloudflare.com/turnstile/plans/) | Needs a browser/webview context to render the widget (not a plain Rust HTTP client): in Tauri it means a small WebView page; or use rate limiting + proof-of-work instead | Enterprise only |
| **Workers Rate Limiting binding** | Per-IP/per-key limits inside the Worker (https://developers.cloudflare.com/workers/runtime-apis/bindings/rate-limit/) | Availability on the Free plan not confirmed from docs I fetched; counters are per location, "permissive", not a strict global count | - |
| **Supabase** | 500 MB DB, 2 active projects, 5 GB egress, **project paused after 1 week of inactivity** (https://uibakery.io/blog/supabase-pricing , third-party summaries) | Pausing breaks an app that is used sporadically (needs a keep-alive cron) | Pro about $25/month (third-party) |
| **Firebase (Spark)** | Firestore 1 GiB, 50k reads/day, 20k writes/day (third-party summary, https://agentdeals.dev/vendor/firebase) | Google lock-in, Blaze (pay-as-you-go) needs a card and can bill | Pay-go |
| **Deno Deploy** | about 1 M requests/month, 1 GiB KV, 450k KV reads, 300k KV writes per month (third-party, https://www.srvrlss.io/provider/deno-deploy/) | KV is eventually consistent, less convenient than SQL for aggregates | Paid plans |
| **Vercel Hobby** | 100k function invocations/month, 100 GB bandwidth, **non-commercial** only (third-party, https://deploywise.dev/blog/vercel-free-tier-limits-2026) | Hobby ToS non-commercial restriction; GPL project is fine but ambiguous if donations | Pro per seat |

- **Best fit if we ever do it: Cloudflare Worker + D1**. One `POST /v1/submit` (validate, upsert row), one `GET /v1/aggregate?score_version=N` (precomputed JSON, cached at the edge, so reads cost ~0 D1 rows). 100k requests/day and 100k row writes/day are way above the plausible volume of a small open-source tool (even 1,000 submissions/day is 1% of limits). Aggregates computed in a cron trigger and stored as a single JSON in KV/R2/Pages so that clients never touch D1.
- **Effort**: medium-high: Worker code (about 200-400 lines TS), schema/migrations, validation, aggregation job, deployment pipeline/secrets, **operations** (monitoring, abuse handling, backups: D1 Time Travel 7 days on free), privacy notice and GDPR role (see 3). Realistic 2-4 weeks for first working version including client; then an ongoing tax.
- **Cost**: 0 on free; $5/month if volume or Workers CPU limit requires it; domain optional (workers.dev subdomain works).
- **Risks**: single maintainer bus factor (service outage or abandoned backend leaves clients with errors; make client tolerant and the leaderboard feature optional); data poisoning (see 2d); legal duties as a data controller.

### 2d. Anonymous HTTPS submission with signed payloads, and why anti-cheat is impossible
- The client is open-source (GPL): anyone can read the code, the signing key, the workloads, and craft a payload. Signatures/HMAC with an embedded key only prove "sent by something that knows the key" (everyone). A per-install key adds nothing without hardware attestation (TPM attestation exists but is overkill, closed-ecosystem and a privacy problem; not viable for a free hobby project).
- Blender (which has even a launcher authenticator token and a closed set of scenes) still has modified-scene and absurd-value issues (links in 1.7); UserBenchmark/PassMark/Geekbench rely on volume and filtering.
- **Mitigations that work for a small project**
  1. **Aggregate by median** per (hardware model, score_version) and require **n >= 5** (Geekbench) before showing a model; show "few samples" badge below that.
  2. **Plausibility check against the reference table / hardware specs**: score must be within a band around the nearest reference or a spec-derived bound (cores x boost clock, memory bandwidth, PCIe generation for disks); reject > e.g. 3x outliers; use MAD/IQR trimming server-side.
  3. **Consistency checks inside the payload**: workload scores must be self-consistent (single <= multi, ratio multi/single <= logical cores x factor), the run must carry telemetry the app already has (clock, temperature, throttling, power plan, background CPU load) and flags like `thermal_throttled`, `on_battery`, `power_saver`, `vm` -> excluded from main aggregate (PassMark excludes overclocked).
  4. **Rate limit** per IP / per anonymous install-id (store only a salted hash, short retention) and one-submission-per-model-per-install dedupe (PassMark filters duplicates from the same system).
  5. **Proof-of-work or Turnstile** to make bulk forging costly (Turnstile needs a webview; PoW such as Altcha-style hashcash can be done natively; I found no source specific to this project, so treat as design option).
  6. **Hardware fingerprint = model strings only** (CPU brand string, GPU name + driver version, disk model, RAM size/speed class): no serial numbers, no MAC, no machine GUID.
  7. **Keep raw submissions and re-aggregate**: if abuse is found, drop a time window or a source and recompute; keep the score_version immutable.
  8. **Don't publish individual entries or usernames**: no leaderboards of "top users" (nothing to cheat for). Only per-model medians + percentile. This kills the incentive for forging; the main remaining incentive is vandalizing a model's median, which needs many forged submissions.
- Bottom line: design for **robustness (median, minimum count, outlier trimming), not for authenticity**.

### 2e. What a submission needs (and personal-data analysis)

Minimal payload (all pseudonymous by design):
```
score_version, app_version, workload_version,
cpu_model (brand string), cpu_cores/threads, base/boost clocks, 
gpu_model, gpu_driver_version, vram,
ram_total_gb, ram_speed_mts, ram_channels,
disk_model, disk_interface/bus (NVMe/SATA/USB), disk_capacity_class, disk free space class,
os_family+build, power_plan, on_battery flag, vm flag, throttling flags,
scores (per workload + composites), run_timestamp (day precision), 
optional: locale-independent country? -> NO (do not send)
```
Do **not** collect: serial numbers, MAC, machine GUID, hostname, Windows user, volume serial, IP (see below), full timestamps, geolocation.

**GDPR analysis (Italy/EU; not legal advice)**
- IP addresses are personal data (Recital 30; CJEU *Breyer* C-582/14 for dynamic IPs when means exist to identify; https://iapp.org/news/a/in-breyer-decision-today-europes-highest-court-rules-on-definition-of-personal-data , https://gdprlocal.com/is-an-ip-address-personal-data/). Any HTTPS endpoint sees IPs. Cloudflare logs IPs at its edge regardless (they are the processor/their own controller for CDN logs); our Worker may simply not store or log them (`CF-Connecting-IP` can be read for rate limiting but never written to D1).
- **Pseudonymised data is still personal data** (Recital 26; EDPB Guidelines 01/2025 on pseudonymisation, https://www.edpb.europa.eu/our-work-tools/documents/public-consultations/2025/guidelines-012025-pseudonymisation_en). A stable per-install ID or a rare hardware combination (e.g. a unique CPU+GPU+disk+RAM+driver set) can identify a person; the more fields, the higher the singling-out risk. Therefore: coarse buckets (RAM to nearest GB class, capacity classes), no per-install persistent ID stored server-side (or a daily-rotating salted hash for rate limiting only), publish only aggregates.
- **ePrivacy Art. 5(3)**: reading information from the user's device for a purpose that is not strictly necessary to a service the user requested requires consent; EDPB Guidelines 2/2023 extend it beyond cookies (https://www.edpb.europa.eu/system/files/2024-10/edpb_guidelines_202302_technical_scope_art_53_eprivacydirective_v2_en_0.pdf). Submitting a benchmark result that the user explicitly triggers with a button is an **explicit user action** (good basis); **never auto-upload or background telemetry**.
- **Legal basis**: consent (Art. 6(1)(a)) via an explicit, per-submission opt-in with a preview of the exact JSON being sent, not pre-ticked, withdrawal possible (delete request by submission ID shown to the user, if stored server-side; for 2b, the public issue deletion). Blender and OpenBenchmarking.org both use opt-in and anonymity by default; Geekbench free auto-uploads publicly (do not copy it).
- **Duties if we run a server or GitHub dataset**: you become the controller: privacy notice (identity/contact of the maintainer, categories of data, purposes, legal basis, retention, recipients incl. Cloudflare/GitHub as processors/recipients, rights, how to request deletion, no transfers or SCC/DPF note for US providers), a record of processing (Art. 30; small-controller exemption is narrow and rarely applies for non-occasional processing), an erasure/contact path, retention limit (e.g. raw submissions deleted after 12-24 months, only aggregates kept), a DPA with the processor (Cloudflare provides a standard DPA), security measures. If the data stays truly anonymous (no IP stored, no ID, coarse hardware), most obligations shrink, but proving anonymisation is hard; assume it is personal data.
- Hobby/personal-project exemptions (household exception) **do not** apply to a public project that collects other people's data.
- GitHub option: submitter's GitHub username and content become public; explicit notice "this is public forever and CC0"; the maintainer is the controller of the dataset repo content.
- A **privacy notice** page in the repo (`PRIVACY.md`, it/en) plus an in-app dialog is enough at this scale; link it from the Share dialog.

## 3. How rankings are shown in UIs
- **Blender Open Data**: ranked list/bars of median scores per device, split CPU/GPU, filter by version and compute backend, with OS/compute distribution charts; your own score is compared against these medians (https://opendata.blender.org/).
- **Geekbench Browser**: ordered chart of processors by average score, per-processor page with distribution of results; your result page shows score vs. other devices (https://browser.geekbench.com/processor-benchmarks).
- **PassMark**: horizontal bar charts per model with the average score ("high-end / mid-range / low-end" charts), sample counts shown per model (https://www.cpubenchmark.net/). In PerformanceTest, system percentile vs baselines [unverified].
- **UserBenchmark**: per-model "effective speed" bar and a percentile-style "speed rank" among the same model's users [from general knowledge].
- **CPU-Z Bench**: your score as a bar compared to a selectable reference CPU, with the reference chosen from a built-in dropdown; the Validator website positions the CPU inside a distribution graph (https://valid.cpuid.com/bench/20).
- **OpenBenchmarking.org / PTS**: percentile of your result among comparable public results, shown in the terminal and on the result page (https://www.phoronix.com/news/OB-Seamless-Comparisons-RFC).
- **3DMark**: "Hall of Fame" top lists plus a text with "higher than X% of all results" [unverified].
- **Pattern to copy for OMA**: a **sorted horizontal bar list** of reference hardware in the same class (e.g. 8-12 rows), **the user's row inserted at its position and highlighted** with the Synthwave accent colour, plus one headline percentile ("faster than 72% of reference hardware") with a "based on N reference entries, score version 1" caption; separate tabs for CPU single, CPU multi, GPU compute, GPU graphics, Disk read/write; a small history sparkline of the user's own runs. Always show sample count and the version, and a "reference measured by project, not from users" label until real data exists. (Our dataviz skill can handle colours/accessibility; not needed for this report.)

## 4. Facts to decide upon (cheat sheet)

| Question | Answer |
|---|---|
| Is a ranking possible with no backend? | Yes: bundled reference table (CPU-Z, Cinebench approach) |
| Is anti-cheat possible with an open client? | No; use medians, min-sample, plausibility, flags |
| Does GitHub scale for a submission flow? | Yes at hobby scale; needs a GitHub account; public and permanent data; use Pages for reads, not raw |
| Does Cloudflare free cover us? | Yes by orders of magnitude (100k req/day, 100k D1 writes/day); safe failure (stops, doesn't bill) |
| Is the data personal under GDPR? | IP: yes; per-install IDs and rare hardware combos: yes (pseudonymous); aggregates by model: no |
| Is consent needed? | Yes, explicit opt-in per submission with preview (ePrivacy 5(3) + Art. 6(1)(a)) |
| Must scores be versioned? | Yes: `score_version`, never mix; Blender/Geekbench/PassMark/CPU-Z all do |
| Which normalization? | Ratio vs a baseline reference machine, geometric mean across workloads (Geekbench) |

## 5. Recommendation

### Option 1 (recommended first): Local ranking with bundled reference table + local history; submission-ready format
- **What**: ship `reference-vN.json` (about 20-40 anchor models per category, measured by the author and friends, later by community PRs), normalized scores relative to a fixed baseline machine, per-category percentile/bar UI (section 3), local history, "Export result (JSON)" button. Score object already includes `score_version`, hardware strings and validity flags so that it is submission-ready.
- **Effort**: about 1-2 weeks on top of the benchmarks themselves (JSON schema, Rust percentile fn with tests, Svelte list/bars, i18n it/en).
- **Recurring cost**: EUR 0; maintenance = occasionally add reference rows per release.
- **Risks**: reference coverage is thin (mitigate with "nearest class" wording, never exact-model claims you can't back); the numbers reflect our workloads only; contributors' machines vary (publish measurement protocol: AC power, High performance plan, idle machine, 3 runs, median).
- **GDPR**: none (nothing sent).

### Option 2 (second, only after the benchmarks are stable for 1-2 releases): GitHub-based opt-in submissions
- **What**: "Share to community dataset" opens a pre-filled GitHub issue (or the user attaches the exported JSON); Action validates against JSON Schema and plausibility vs reference; auto-merges to `data/submissions/`; weekly Action recomputes medians per (model, score_version, n >= 5) into `aggregate-vN.json` on GitHub Pages; app fetches it at most once a day (ETag) and merges it with the bundled table (bundled table remains the fallback). CC0 licence + `PRIVACY.md`.
- **Effort**: about 1-2 weeks. **Cost**: 0. **Risks**: low participation, public permanent data (inform users), moderation by hand for outliers, Action maintenance, GitHub account barrier. Rate limits irrelevant if clients read Pages once a day.
- **GDPR**: manageable with explicit preview and CC0 notice; no IP, no IDs; erasure = delete issue/commit entries manually (document this honestly).

### Option 3 (only if Option 2 shows real demand): Cloudflare Worker + D1 endpoint
- **What**: `POST /v1/submit` + cached `GET /v1/aggregate`; per-IP rate limit, optional PoW/Turnstile, server-side validation/trimming; raw rows retained 12 months; aggregate JSON cached at edge; client falls back to bundled table when offline or when the service is gone.
- **Effort**: 2-4 weeks first version, plus ongoing (monitoring, abuse, privacy requests). **Cost**: EUR 0 free tier (maybe $5/month later). **Risks**: you become a data controller with real duties; single-maintainer bus factor; poisoning; the free D1 daily cap makes a bad actor able to exhaust writes (acceptable: reads served from cache).
- Only worthwhile if you want a live "percentile among all OMA users", and if you accept the privacy-notice/DPA workload.

### Phased path
1. **Phase A (ship with the benchmark feature)**: Option 1. Define `score_version = 1`, baseline machine, workloads and normalization formula in a public doc (`docs/benchmark-scoring.md`) to avoid the UserBenchmark credibility trap. Add validity flags (throttling, battery, power plan, VM).
2. **Phase B (1-2 releases later)**: add export/"copy result" and a documented, community **PR-based** way to add reference rows (this is Option 2 minus automation, costs nothing); write `PRIVACY.md` even if nothing is collected yet.
3. **Phase C (on demand)**: Option 2 automation (issue form + Actions + Pages aggregate), per-version datasets.
4. **Phase D (optional)**: Option 3 only if Phase C aggregates show hundreds of submissions and users ask for live percentiles; reuse the same JSON schema and aggregation code (run it in the Worker), so the migration is transport-only.
5. Never merge scores across `score_version`; when workloads change, bump the version, re-measure reference rows, and keep the old table labelled legacy for at least one release.

### Decision
Drop the **online** leaderboard from the first benchmark release; do **not** drop the ranking. The local reference-table approach gives users 90% of the value (a position among known hardware), has zero privacy burden, and keeps the door open to Options 2 and 3 because the score format is versioned and submission-ready.

## Sources (main)
- Blender Open Data: https://opendata.blender.org/ , https://opendata.blender.org/about/ , https://projects.blender.org/infrastructure/blender-open-data , https://www.blender.org/news/introducing-blender-benchmark/ , issues: https://projects.blender.org/archive/blender-benchmark-bundle/issues/76290 , https://projects.blender.org/archive/blender-benchmark-bundle/issues/96519 , https://projects.blender.org/infrastructure/blender-open-data/issues/73322
- Geekbench: https://www.geekbench.com/doc/geekbench6-benchmark-internals.pdf , https://browser.geekbench.com/processor-benchmarks , https://primatelabs.com/policy/prerelease-hardware.html
- PassMark: https://www.cpubenchmark.net/cpu_test_info.html , https://forums.passmark.com/pc-hardware-and-benchmarks/48456-how-are-average-cpu-benchmarks-calculated
- UserBenchmark: https://en.wikipedia.org/wiki/UserBenchmark , https://www.tomshardware.com/news/userbenchmark-benchmark-change-criticism-amd-intel%2C40032.html
- 3DMark Hall of Fame: https://benchmarks.ul.com/hall-of-fame-2/fire+strike+3dmark+score+extreme+preset/version+1.0/1+gpu
- CPU-Z: https://valid.cpuid.com/bench/20 , https://www.techradar.com/how-to/how-to-use-cpu-z
- OpenBenchmarking.org: https://www.phoronix.com/news/OB-Seamless-Comparisons-RFC , https://mail.openbenchmarking.org
- Cinebench tables: https://nanoreview.net/en/cpu-list/cinebench-scores
- GitHub limits: https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api , https://github.blog/changelog/2025-05-08-updated-rate-limits-for-unauthenticated-requests/ , https://docs.github.com/en/pages/getting-started-with-github-pages/github-pages-limits , https://github.com/morpheus65535/bazarr/issues/3057
- Cloudflare: https://developers.cloudflare.com/workers/platform/limits/ , https://developers.cloudflare.com/workers/platform/pricing/ , https://developers.cloudflare.com/d1/platform/pricing/ , https://developers.cloudflare.com/d1/platform/limits/ , https://developers.cloudflare.com/turnstile/plans/ , https://developers.cloudflare.com/workers/runtime-apis/bindings/rate-limit/
- Other free tiers (third-party summaries): https://uibakery.io/blog/supabase-pricing , https://agentdeals.dev/vendor/firebase , https://www.srvrlss.io/provider/deno-deploy/ , https://deploywise.dev/blog/vercel-free-tier-limits-2026
- GDPR: https://iapp.org/news/a/in-breyer-decision-today-europes-highest-court-rules-on-definition-of-personal-data , https://gdprlocal.com/is-an-ip-address-personal-data/ , https://www.edpb.europa.eu/our-work-tools/documents/public-consultations/2025/guidelines-012025-pseudonymisation_en , https://www.edpb.europa.eu/system/files/2024-10/edpb_guidelines_202302_technical_scope_art_53_eprivacydirective_v2_en_0.pdf
- Database right: https://www.jacobacci-law.com/news-and-events/cjeu-c-762-19-the-sui-generis-right-of-the-database-maker
