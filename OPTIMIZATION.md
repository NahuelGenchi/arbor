# Arbor: optimization report

16 Oct 2026 · input for the first prototype. **M** = measured on this machine; every other number links to its source; **est.** = estimate.
Machine: i7‑14700KF (8P+12E cores), 15 GiB RAM, NVMe SSD, Radeon R9 380, ASUS PRIME B760M‑A, Ubuntu 26.04, Linux 7.0. Agent data: your 201 local Claude Code sessions (47.7k model calls).

## Where the waste is today (M)

- **Tokens:** 74% of agent spend is re-reading context. One step costs ≈42k input-token-equivalents; the median tool output is 209 tokens. Cut steps and context before output size.
- **RAM:** agent runtimes and UIs, not the OS: 6 Claude Code sessions 1,568 MiB, Electron UI 1,354 MiB, headless Chrome 2,054 MiB, GNOME 513 MiB (RSS). Mid-investigation: 9.6 GiB swapped out (≈6 GiB of it to disk), 0.9 GiB RAM available.
- **Disk:** 32.5% of your 70.5 GiB projects folder is byte-identical copies (6.8 GiB of it agent worktrees, 7.2 GiB build dirs).
- **Boot:** 25.2 s; firmware alone is 11.3 s (45%). The initramfs carries 1,099 kernel modules; this PC loads 52 of them.
- **Search:** `find` over 2.2M entries takes 1.4 s (warm cache); an index answers in 0.3 ms.

## Top 10 for the prototype

All buildable on Linux in phase 1, ordered by payoff on your data. Sources are in the sections below.

| # | Change | Before → after |
|---|---|---|
| 1 | Tool schemas on demand, a stable cached prompt prefix, stale tool output expired | 70k-token opening context re-read every step = 23% of cache reads, ≈1/6 of spend (M); tool search: 77k → 8.7k tokens with accuracy 49% → 74%; expiring old output: −84% tokens |
| 2 | Typed results printed as tables of the asked fields; many reads per step | file listing 6,283 → 401 tokens, `git log` 18,928 → 530 (M); 16% of steps just follow another read (M) |
| 3 | Worlds = copy-on-write snapshots over a content-addressed store | fork a 28 GB project with its build outputs in **4.3 ms** (M); disk −32.5% (M) |
| 4 | Cells = namespaces forked from warm templates; Wasm or snapshot microVMs for untrusted code | start 125 ms → 1.28 ms; Python cell 46.4 ms / 15.2 MiB → **0.73 ms / 2.1 MiB** (M) |
| 5 | One native agent service (agents are handles); no browser engine per app | 250–334 MiB per agent (M) → a few MiB (est.); UI stack 1.9 GiB (M) → one compositor |
| 6 | Compressed RAM instead of disk swap; cold pages compressed early | ≈6 GiB of disk swap (M) → 0; zram holds 2.0× here (M); −20% to −32% memory at Meta |
| 7 | Live catalog, trigram index, outline-first code objects | `find` 1,425 → **0.32 ms**; code search 37–174 → 1.4–5.3 ms; read 4–8% of a large file (M) |
| 8 | One kernel image, no initramfs, no splash, no boot-time jobs; resume or kexec instead of rebooting | 25.2 s (M) → ≈13 s on this board, ≈2.5 s with coreboot-class firmware (est.) |
| 9 | Nothing polls; background work batched on E-cores; per-cell energy preference; device idle states on | background CPU power up to −90%; P/E placement −8% to −27% energy; ASPM ≈−5 W, NVMe ≈−1 W |
| 10 | Profile-guided builds (PGO + LTO + BOLT) of the kernel and hot services | up to +8% (services), +20% (compilers), +5% (kernel) on the same hardware |

## 1. RAM, CPU, GPU

| Change | Before → after | Evidence |
|---|---|---|
| **Cells are namespaces forked from warm templates**, not containers | start: `docker run` 125 ms → 1.28 ms with user+net+pid+mnt namespaces. A Python cell with its imports: 46.4 ms and 15.2 MiB → **0.73 ms and 2.1 MiB** when forked from a warm template. Templates: provisioning 18× faster than Docker, then another 3× | M, [SOCK, ATC'18](https://www.usenix.org/conference/atc18/presentation/oakes) |
| **Untrusted code runs in Wasm cells or snapshot-restored microVMs** | Wasm, pre-initialized: start 2.8 s (Docker) → 0.5 ms, memory 1.3 MB → 90 KB PSS per cell; copy-on-write heap images: instantiation 2 ms → 5 µs. MicroVM: VMM 131 MB (QEMU) → 3 MB (Firecracker), restore 4–10 ms, whole sandbox <20 MB with shared read-only images | [Faasm, ATC'20](https://ar5iv.labs.arxiv.org/html/2002.09344), [Wasmtime 2022](https://bytecodealliance.org/articles/wasmtime-10-performance), [Firecracker, NSDI'20](https://www.usenix.org/system/files/nsdi20-paper-agache.pdf), [Brooker 2022](https://brooker.co.za/blog/2022/11/29/snapstart/), [RunD, ATC'22](https://www.usenix.org/system/files/atc22-li-zijun-rund.pdf) |
| **One native agent service; each agent is a handle**, not a process | a Claude Code session uses 250–334 MiB RSS to hold a median context of 266k tokens ≈ 1 MB of text. Idle floor: Node 41.7 MiB, Python 9.6, Rust 2.2, C 0.6; start: Node 12.8 ms, Rust 0.42 ms. CPU-bound energy vs C: Rust 1.03×, JavaScript 4.45×, Python 75.9× | M, [Pereira, SLE'17](https://greenlab.di.uminho.pt/wp-content/uploads/2017/10/sleFinal.pdf) |
| **Compressed RAM instead of disk swap; cold pages compressed before pressure** | today: an 8 GiB disk swap file next to 4 GiB of zram; ≈6 GiB went to disk during this investigation. zram holds 2.0× here, and agent heap pages compress 2.15× with zstd. Proactive cold-page offload: −20% to −32% of memory at Meta, with no noticeable slowdown | M, [Meta TMO 2022](https://engineering.fb.com/2022/06/20/data-infrastructure/transparent-memory-offloading-more-memory-at-a-fraction-of-the-cost-and-power/) |
| **The compositor draws Mission Control; one shared web engine, only for web pages** | today: Electron UI 1,354 MiB + GNOME 513 MiB + headless Chrome 2,054 MiB; gnome-shell uses 2.1% of a core on a still screen. A system webview in place of Electron cuts RSS only 11–31%, so the gain has to come from removing engines | M, [Tauri benchmarks](https://raw.githubusercontent.com/tauri-apps/benchmark_results/gh-pages/tauri-data-linux.json) |
| **Video and static layers on display planes** (the GPU stops compositing them) | playback power −7% to −14% (laptop, 4K and 1080p) | [KWin 2025](https://zamundaaa.github.io/wayland/2025/10/23/more-kms-offloading.html) |

Keep (already on here): MGLRU page reclaim (Google fleet: kswapd CPU −40%, low-memory kills −85%; [LWN 2022](https://lwn.net/Articles/908512/)) and transparent huge pages in madvise mode.

## 2. Storage and search

| Change | Before → after | Evidence |
|---|---|---|
| **Content-addressed objects**: one copy of each content across all worlds (whole objects first, sub-file chunks later) | your projects folder: 70.5 → 47.6 GiB (−32.5%); agent transcripts −38%. Whole-file dedup gets ~¾ of block-level savings | M, [Meyer & Bolosky, FAST'11](https://www.usenix.org/legacy/event/fast11/tech/full_papers/Meyer.pdf) |
| **Worlds are copy-on-write snapshots, fetched lazily** | your 28 GB AnyPS5 tree forked with its build outputs in **4.3 ms** (`git worktree`: 79 ms for a 19 MB checkout that then rebuilds 1,774 steps). Copy 14.8–55 s → snapshot 0.4 s. Container pulls are 76% of start time, yet only 6.4% of the pulled data is read | M, [FAST'20](https://www.usenix.org/system/files/fast20-zhan.pdf), [Slacker, FAST'16](https://www.usenix.org/system/files/conference/fast16/fast16-papers-harter.pdf) |
| **Live catalog** of every object (id, parent, name, type, hash), updated from the store's change log after each write: no crawler, no `updatedb`, no per-folder watches | `find` over your 2.23M entries: 1,425 ms → **0.32 ms**; exact name 0.05 µs; 90 B per object (191 MiB); 2 µs per change. plocate: 20.1 s → 8 ms on 27M files; `git status` with a change feed: 85 s → 0.75 s. Synchronous indexes cost ~50% on metadata-heavy work | M, [plocate](https://plocate.sesse.net/), [GitHub 2022](https://github.blog/engineering/infrastructure/improve-git-monorepo-performance-with-a-file-system-monitor/), [Giampaolo 1999](http://www.nobius.org/dbg/practical-file-system-design.pdf) |
| **Trigram index** for text and code (regex-capable) | `rg` over 248 MiB of your code: 37–174 ms → **1.4–5.3 ms**. Linux tree: 1.96 s → 0.01 s, with an index ≈20% of the source size | M, [Cox 2012](https://swtch.com/~rsc/regexp/regexp4.html) |
| **Code objects know their structure**: outline first, then functions by name | 40 large files (median 40k tokens): whole file → outline **4%**, outline + 2 functions **8%** of the tokens | M |
| **Semantic index** (HNSW in RAM, DiskANN on SSD), built off the write path | 1M vectors: 0.02 ms per query at 0.95 recall; 1B vectors on one 64 GB machine: <3 ms at ≥95% recall | [Faiss](https://github.com/facebookresearch/faiss/wiki/Indexing-1M-vectors), [DiskANN](https://proceedings.neurips.cc/paper_files/paper/2019/file/09853c7fb1d3f8ee67a61b6bf4a7f8e6-Paper.pdf) |
| **Small objects packed** into append-only volumes with an in-RAM index | disk reads per object ≈3 → 1; index RAM 10 B per object vs 536 B per XFS inode | [Haystack, OSDI'10](https://www.usenix.org/legacy/event/osdi10/tech/full_papers/Beaver.pdf) |

## 3. Boot

Today (M): **25.2 s** = firmware 11.3 + boot loader 1.7 + kernel and initramfs 6.5 + userspace 5.7.

| Change | Before → after | Evidence |
|---|---|---|
| **Resume instead of booting**: suspend-to-idle for daily use, kexec into kernel updates | kexec skips firmware and loader: **13.0 s** of this PC's 25.2 s. Suspend-to-idle also skips firmware on resume (S3 resume: 2,087 ms, 340 ms of it firmware) | M, [Brown 2015](https://events.static.linuxfound.org/sites/events/files/slides/Brown-Linux-Suspend-at-Speed-of-Light-LC-EU-2015.pdf) |
| **Firmware that starts only what the OS uses** (coreboot/LinuxBoot class, where the board supports it) | firmware 11.3 s here → **1.14 s** from power-on to kernel on a coreboot laptop; a server's UEFI: ~8 min → ~20 s with a Linux payload | M, [LKML 2023](https://lkml.iu.edu/hypermail/linux/kernel/2310.0/00408.html), [LWN 2017](https://lwn.net/Articles/738649/) |
| **One kernel image, no boot menu, no generic initramfs**: this PC's storage and filesystem drivers built in, everything else loaded on demand | loader 1.7 s → firmware starts the kernel directly. Today's 90.7 MB initramfs carries **1,099 modules, of which this PC loads 52** (1.6 of 33.8 MB), 17.2 MB of firmware for datacenter NICs, 15.1 MB of microcode for every Intel and AMD CPU, and 63 MB of udev/plymouth userspace | M |
| **Userspace starts the first screen only**: no splash, nothing waits for the network, update checks and crawlers run later as background work | userspace 5.7 s; the splash screen's exit wait alone adds **+2.04 s** on the critical path. Jobs at boot: ua-timer 3.9 s, apt-daily-upgrade 1.4 s, snapd 1.6 s, fwupd 0.5 s, man-db 0.3 s | M |

Already done here, so no gain is left: zstd kernel and initramfs, parallel CPU bring-up, `quiet`.
Target (est.: kernel ≈1 s as on the coreboot laptop, first screen ≈0.5 s): **≈13 s** with this board's vendor firmware (firmware is then ~88% of boot), **≈2.5 s** with coreboot-class firmware.

## 4. More speed from the same hardware

| Change | Before → after | Evidence |
|---|---|---|
| **Profile-guided builds** of the kernel and hot services (PGO + LTO, then BOLT) | BOLT on top of PGO + LTO: up to +8.0% on data-center services, +20.4% on GCC/Clang. Kernel with AutoFDO + Propeller: up to +5% on large benchmarks | [BOLT 2018](https://arxiv.org/abs/1807.06735), [LKML 2024](https://lkml.iu.edu/hypermail/linux/kernel/2411.0/01852.html) |
| **Hugepage-aware allocator** for long-running services; 64 KB folios for anonymous memory | requests/s +7.7% and RAM −2.4% (TCMalloc Temeraire, 8 apps); kernel build −5% wall time with 64 KB folios | [OSDI'21](https://www.usenix.org/system/files/osdi21-hunter.pdf), [LKML 2023](https://lkml.iu.edu/hypermail/linux/kernel/2307.3/04006.html) |
| **Batched async system calls** (io_uring) for the store, catalog and outbox, **never reachable from cells** | batching syscalls without traps: Apache +116%, MySQL +40%. But 60% of kernel exploits submitted to Google's kCTF used io_uring, and Google disabled it on its production servers | [FlexSC, OSDI'10](https://www.usenix.org/legacy/events/osdi10/tech/full_papers/Soares.pdf), [Google 2023](https://security.googleblog.com/2023/06/learnings-from-kctf-vrps-42-linux.html) |
| Zoned (ZNS) SSDs, once Arbor ships its own hardware | sustained writes 370–590 → 1,010 MiB/s; write amplification 2.0–2.4× → ~1.0× | [ZNS, ATC'21](https://www.usenix.org/conference/atc21/presentation/bjorling) |

Low payoff: x86-64-v3 builds (supported by this CPU) gain ~1% for most packages ([Canonical 2025](https://discourse.ubuntu.com/t/introducing-architecture-variants-amd64v3-now-available-in-ubuntu-25-10/71312)).

## 5. Energy at the same quality

| Change | Before → after | Evidence |
|---|---|---|
| **Nothing polls**: tickless kernel, event-driven services, timers coalesced by class (UI ≈1 ms slack, daemons ≈100 ms) | without tickless idle a battery drains 2–3× faster; timer coalescing cut idle CPU activity 72% (Apple's test, reported by press) | [kernel docs](https://docs.kernel.org/timers/no_hz.html), [iMore 2013](https://imore.com/mavericks-preview-timer-coalescing) |
| **Background QoS class**: indexing, sync and ledger compaction run batched on efficient cores | CPU power up to −90%, under half the CPU energy for the same work; deferring idle-time work: +30% screen-off battery | [EcoQoS 2021](https://devblogs.microsoft.com/performance-diagnostics/introducing-ecoqos/), [Android Doze](https://sdtimes.com/android/android-marshmallow-arrives/) |
| **Energy preference and core placement per cell** (this PC runs EPP = *performance* on every core; sched_ext is built in but off) | balance_power: −52% power for video playback; energy-aware P/E placement: −8% to −27% energy for +0.7–2.4% runtime; sched_ext LAVD: −16% energy at the same frame rate | M, [LKML 2024](https://lkml.iu.edu/hypermail/linux/kernel/2406.0/08964.html), [EAS, LKML 2018](https://lkml.rescloud.iu.edu/1812.0/00841.html), [LPC 2024](https://lpc.events/event/18/contributions/1713/attachments/1425/3058/scx_lavd-lpc-mc-24.pdf) |
| **Still screen = no work**: the UI repaints only on change, the panel drops to a low refresh rate, hidden views get no timers | variable refresh: −22% panel power, −60% on still images at 10 Hz; sleeping hidden tabs: CPU −37%, RAM −32% | [Samsung 2020](https://news.samsung.com/us/samsung-display-commercializes-new-variable-refresh-rate-technology-for-increasing-smartphone-power-savings-galaxy-note20/), [Edge 2020](https://blogs.windows.com/msedgedev/?p=24627) |
| **Device idle states on by default**, with the safe policies (ASPM, NVMe APST, SATA med_power_with_dipm, Wi-Fi power save) | ≈−5 W (ASPM), ≈−1 W (APST), −0.5 to −3 W (SATA), Wi-Fi radio 1,120 → 72 mW | [ASPM](https://www.kernel.org/pub/linux/kernel/v3.x/ChangeLog-3.2.5), [APST](https://lists.infradead.org/pipermail/linux-nvme/2016-August/006056.html), [Fedora 28](https://fedoraproject.org/wiki/Changes/ImprovedLaptopBatteryLife), [NAPman](https://microsoft.com/en-us/research/wp-content/uploads/2016/02/NAPman.pdf) |
| **Media on fixed-function hardware** | decode energy → 4.4–8.9% of optimized software decoding | [Kränzler 2024](https://arxiv.org/abs/2402.09001) |
| **The local model service batches requests and uses the NPU** for prefill and embeddings | energy per request −12.5× (batching) to −100× (batching + spacing arrivals) vs one at a time; NPU prefill uses 1.85–4.3× less energy than the GPU | [Delavande 2026](https://arxiv.org/abs/2601.22362), [llm.npu](https://arxiv.org/html/2407.05858v2) |
| On battery or cellular: batch background network traffic (never interactive streams) | network energy −35% to −52% | [TailEnder, IMC'09](https://people.cs.umass.edu/~arun/papers/TailEnder.pdf) |

## 6. Agent tokens: cut steps and context first

| Change | Before → after | Evidence |
|---|---|---|
| **Prompt/KV cache as a system service**: stable prefix first, volatile data last, no timestamps in the prefix, shared across agents | your agents already read 98.7% of input from cache: **−87%** spend vs uncached. Cache reads bill at 0.1×; a 100k-token prompt: first token 11.5 s → 2.4 s | M, [docs](https://platform.claude.com/docs/en/build-with-claude/prompt-caching), [launch post](https://claude.com/blog/prompt-caching) |
| **Tool schemas on demand**: services describe themselves; agents search them | your opening context: median **70k tokens = 23%** of everything re-read; each 10k removed ≈ −2.3% of spend. Tool search: 77k → 8.7k tokens (−85%), accuracy 49% → 74% | M, [Anthropic 2025](https://www.anthropic.com/engineering/advanced-tool-use) |
| **One step, many reads**: typed batch queries, and scripts that call tools without a model round trip | **16%** of your steps (13% of spend) directly follow another read-only step. Programmatic calls: 43.6k → 27.3k tokens (−37%) with higher accuracy; code execution: 150k → 2k on one task | M, [Anthropic 2025](https://www.anthropic.com/engineering/advanced-tool-use), [MCP code](https://www.anthropic.com/engineering/code-execution-with-mcp) |
| **Expire stale tool output** (keep recent observations, drop old ones) | −84% tokens with +29% task score (+39% with memory); masking old observations halves cost at about the same solve rate | [context mgmt](https://claude.com/blog/context-management), [JetBrains 2025](https://arxiv.org/abs/2508.21433) |
| **Typed results printed as tables of only the asked fields; code stays raw text** | 100-file listing: pretty JSON 6,283 → table 2,916 → names only **401** tokens; `git log -20` 18,928 → 530; `ps aux` 9,900 → 286. Tables vs JSON: −42.6% tokens at equal accuracy. Code wrapped in JSON scored at or below plain text for every model tested | M, [TOON](https://github.com/toon-format/toon), [aider 2024](https://aider.chat/2024/08/14/code-in-json.html) |
| **Effort and model routing behind verifiers** | medium effort matched the best prior score with −76% output tokens; cascades: −59% to −98% cost at equal accuracy | [Opus 4.5](https://www.anthropic.com/news/claude-opus-4-5), [FrugalGPT](https://arxiv.org/abs/2305.05176) |
| **Typed app state; screenshots only for visual checks** | your 3,461 screenshots ≈ 2.9–3.9M tokens; one 1080p screenshot = 1,560 tokens | M, [vision docs](https://platform.claude.com/docs/en/build-with-claude/vision) |
| If Arbor hosts models: radix prefix sharing, paged KV, lossless speculative decoding, FP8 KV cache | up to 6.4× throughput; 2–4× throughput (KV waste 62–80% → ~4%); 2–3× faster with identical output; KV memory −50% with ≥97% accuracy recovery | [SGLang](https://arxiv.org/abs/2312.07104), [vLLM](https://arxiv.org/abs/2309.06180), [Leviathan](https://arxiv.org/abs/2211.17192), [vLLM FP8](https://vllm.ai/blog/2026-04-22-fp8-kvcache) |

Low payoff on your data (M): de-duplicating repeat reads (2% of reads), retry avoidance (1.8% of tool calls failed), connection tuning (TLS setup is 12 ms of a 185 ms API call).

## 7. Network

Agent traffic is small HTTPS/SSE calls over 10–150 ms round trips, so the gains come from not repeating setup work.

| Change | Before → after | Evidence |
|---|---|---|
| **The outbox keeps one connection pool for all cells** (keep-alive, HTTP/2 multiplexing, keyed by origin and credential) | connection reuse 87.1% → 99.92% (160× fewer new connections), p95 time to first byte −80 ms. Here a new connection costs 12 ms of a 185 ms API call, more on long paths | [Pingora 2022](https://blog.cloudflare.com/how-we-built-pingora-the-proxy-that-connects-cloudflare-to-the-internet/), M |
| **TLS 1.3 + client session tickets** | cold setup 237.9 → 190.1 ms at 50 ms RTT (−20%), 439.1 → 340.8 ms at 100 ms (−22%); resumption cuts client crypto 9.22 → 2.38 ms per connection | [Sy 2019](https://arxiv.org/pdf/1902.02531) |
| **DNS cache in the outbox** | each avoided lookup saves 2.5–34 ms (resolver hit) or 24–156 ms (miss) | [Al-Dalky 2020](https://ar5iv.arxiv.org/html/2007.00651) |
| **Large windows for bulk transfers** (HTTP/2 window autotuning; TCP buffer max ≥ bandwidth × RTT) | HTTP/2 uploads ran at ~½ the speed of HTTP/1.1 until autotuned, then almost 3× faster; a 3 MB window caps a 100 ms path at 240 Mbps | [Cloudflare 2020](https://blog.cloudflare.com/delivering-http-2-upload-speed-improvements/), [IETF 97](https://www.ietf.org/proceedings/97/slides/slides-97-iccrg-bbr-congestion-control-02.pdf) |
| **Policy in hash maps, filtering early, no connection tracking on the cell → outbox hop** | rule update 15–75 ms → 0.003 ms; per-core drop rate 0.6M → 10.1M packets/s (matters only under floods) | [Miano 2019](https://ccronline.sigcomm.org/wp-content/uploads/2019/07/acmdl19-304.pdf), [Cloudflare 2018](https://blog.cloudflare.com/how-to-drop-10-million-packets/) |

Only on lossy or long paths: HTTP/3 (search p99 −14% to −17%, [SIGCOMM'17](https://static.googleusercontent.com/media/research.google.com/en//pubs/archive/46403.pdf)) and BBR. Datacenter techniques (kernel bypass, zero-copy, BIG TCP) change nothing at agent traffic rates.

## Remove (today's cost on this PC, M)

- Generic initramfs: 90.7 MB, and 1,047 of its 1,099 modules are unused here. Unused modules on disk: 172 MB in 6,903 files; 187 are loaded (2.7%).
- Boot splash (plymouth): +2.04 s on the boot critical path.
- Boot-time jobs (ua-timer 3.9 s, apt-daily-upgrade 1.4 s, fwupd 0.5 s, man-db 0.3 s): move them to the background class.
- snapd: 1.6 s at boot and 14 squashfs loop mounts.
- Crawlers (`updatedb`, file indexers): replaced by the store's change log.
- Disk swap: an 8 GiB file, ≈6 GiB of it in use during this investigation.
- Docker for cells: 125 ms per start vs 1.28 ms for namespaces.
- One runtime process per agent: 250–334 MiB each.
- One browser engine per app (Electron UI 1,354 MiB) and the GNOME shell stack (64 processes, 513 MiB, 2.1% of a core on a still screen).
- Human-format command output and preloaded tool schemas as agent defaults: an 18,928-token `git log`, a 70k-token opening context.

## Don't

- **Disable CPU mitigations:** no real-world gain on current CPUs (Zen 5: none across 140+ benchmarks), and the attacks come back. This CPU isn't affected by Meltdown and uses eIBRS for Spectre v2 (M). [Phoronix 2024](https://www.phoronix.com/review/amd-zen5-mitigations-off/5)
- **Undervolt, or let software reach voltage control:** Plundervolt and CLKSCREW broke SGX and TrustZone this way. [plundervolt.com](https://plundervolt.com/)
- **Merge pages between agents with KSM:** only 2% of non-zero pages are identical across 4 agents here (M); ksmd burns 30–60% of a core; Flip Feng Shui used dedup to break keys. [LWN 2023](https://lwn.net/Articles/953141), [USENIX Sec'16](https://www.usenix.org/conference/usenixsecurity16/technical-sessions/presentation/razavi)
- **Set THP to always:** 2 MB pages bloat RSS; keep madvise. [kernel docs](https://docs.kernel.org/admin-guide/mm/transhuge.html)
- **Use a user-space kernel for cells (gVisor-style):** open+close 216× slower, simple syscalls ≥2.2× slower (2019). [HotCloud'19](https://www.usenix.org/system/files/hotcloud19-paper-young.pdf)
- **Compress prompts lossily:** BBH −8.5 points at 5×, −13.2 at 7× (LLMLingua). [arXiv 2023](https://arxiv.org/abs/2310.05736)
- **Cache answers by similarity:** false-positive hits and substantial degradation; key caches by exact content hash. [arXiv 2025](https://arxiv.org/abs/2506.14852)
- **Fan out agents without a verifier:** multi-agent systems use ≈15× the tokens of chat. [Anthropic 2025](https://www.anthropic.com/engineering/multi-agent-research-system)
- **Lean on long contexts:** at 32k tokens, 11 of 13 models fall below 50% of their short-context score. [NoLiMa](https://arxiv.org/abs/2502.05167)
- **Quantize to int8 with bitsandbytes to save energy:** it used 2–3× more energy than fp32 in decode. [Delavande 2026](https://arxiv.org/abs/2601.22362)
- **Send model calls as 0-RTT:** early data is replayable ([RFC 8446](https://www.rfc-editor.org/rfc/rfc8446.html)). **Use QUIC everywhere:** up to −45% throughput on fast links ([WWW'24](https://arxiv.org/abs/2310.09423)). **Use BBR v1 everywhere:** it crowds out CUBIC in deep buffers ([IMC'19](https://blog.apnic.net/2020/01/24/modelling-bbrs-interactions-with-loss-based-congestion-control/)).
- **Index synchronously on the write path:** costs ~50% on metadata-heavy work (see §2).
