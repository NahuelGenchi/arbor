# Arbor baseline suite

Re-measures, on today's Ubuntu install, the numbers marked **M** in [../OPTIMIZATION.md](../OPTIMIZATION.md), and
measures the Arbor prototype in [../prototype](../prototype) the same way on the same PC, so the two can be compared row
by row.

```bash
./run.sh
```

One run takes about 2½ minutes on the M machine (i7-14700KF, 15 GiB RAM), and about 2 more for the `screen` section,
during which the PC should be left alone. It writes three files to `results/`:

| File | What |
|---|---|
| `<date>_<host>.md` | the report: machine stamp, then one table with every metric next to its M value |
| `<date>_<host>.json` | the same rows with keys and raw context, for diffing two runs |
| `<date>_<host>.log` | everything the sections printed, one JSON record per measurement |

git ignores `results/`: a report names the machine, its hardware and the programs running on it, so runs stay local.

Run a subset with `ONLY="boot search" ./run.sh`, or leave sections out with `SKIP="storage" ./run.sh`. Sections:
`memstate boot runtimes cells pycell search heap transcripts formats storage arbor screen`. One more, `toolkits`,
runs only when named, `ONLY=toolkits ./run.sh`: it builds the test program of issue #8 in each candidate toolkit
([toolkits/README.md](toolkits/README.md)) and measures each one for about four minutes, with nobody at the PC.

`./check.sh` only checks that every script parses (`bash -n`, `py_compile`, `node --check`, `gcc -fsyntax-only`); it
measures nothing and reads no personal data. The Checks workflow runs it on every PR.

## Parameters

Every path and input is set in [config.sh](config.sh) and can be overridden from the environment
(`NAME_QUERY=report ./run.sh`). Nothing personal is hard-coded: defaults derive from `$HOME` and from where `bench/`
sits.

| Variable | Default | Used by |
|---|---|---|
| `PROJECTS_DIR` | the folder holding this project (`bench/../..`) | defaults below |
| `NAME_SEARCH_ROOT`, `NAME_QUERY` | `$HOME`, `invoice` | name search |
| `CODE_ROOT`, `CODE_QUERIES` | `$PROJECTS_DIR`, the five M queries (`\|`-separated) | code search |
| `DEDUP_ROOT` | `$PROJECTS_DIR` | duplicate bytes |
| `FORK_TREE` | largest directory directly under `$PROJECTS_DIR` | overlay forks |
| `TRANSCRIPTS_DIR` | `$HOME/.claude/projects` | token accounting, heap input |
| `HEAP_JSONL`, `HEAP_JSONL_BYTES` | the transcript of exactly the M input's size, else the 3rd largest | heap |
| `O200K_FILE` | the o200k_base.tiktoken shipped with VS Code's Copilot extension | token counts |
| `LISTING_DIR`, `FORMATS_LOCALE` | `/usr/bin`, `en_US.UTF-8` | output formats |
| `GIT_LOG_REPO`, `GIT_LOG_REF` | the repo under `$PROJECTS_DIR` holding the M commit, that commit | `git log` tokens |
| `DOCKER_IMAGE` | `ubuntu:24.04` (must already be present; never pulled) | docker cell |
| `INITRD` | `/boot/initrd.img` | initramfs breakdown |
| `ARBOR_BIN` | empty: a release build of `../prototype` in the temp dir | the prototype |
| `OUTLINE_REF`, `OUTLINE_FILES` | `6bbbb22`, the commit that merged `arbor look`; `10` | a large file read in parts |
| `SCREEN_STILL_S`, `FRAMES_S` | `60`, `20` (seconds) | still screen; each frame-pacing run |
| `FRAMES_CELLS`, `FRAMES_SPIKE_MIB` | empty: one busy cell per CPU thread, and a spike of all the available memory but 1 GiB | the loaded frame-pacing run |
| `TOOLKITS`, `TOOLKITS_PREBUILT` | `gtk4 qtquick slint iced`, in that order; empty: build them into the temp dir | toolkits |
| `TOOLKITS_LAUNCHES`, `TOOLKITS_STILL_S`, `TOOLKITS_SETTLE_S` | `5` launches timed for the first frame; `60` s of still screen; `90` s to settle after the builds and after each loaded run, and then until the 1-minute load average is under 1, at most 10 min more | toolkits |
| `BENCH_TMP`, `RESULTS_DIR` | `$TMPDIR` or `/tmp`, `bench/results` | temp files, reports |

Inputs that keep growing are pinned by content where M used a fixed one: `git log` starts at the commit M saw, and the
heap test loads the transcript of M's exact byte size. The files read in parts are this repository's at a fixed commit.

## What it needs and what it touches

- **No root.** Namespaces are unprivileged user namespaces. The overlay is mounted inside `unshare -Urm`, or by bwrap
  in its sandbox, so it exists only in a private mount namespace and vanishes with it.
- **Temp files** live in one `mktemp -d` directory under `$BENCH_TMP` (tmpfs, as in the M run) and are removed on
  exit, also on Ctrl-C or a section timeout. Docker runs with `--rm --pull never --network none`. Python runs with
  `-I -B`, so no `__pycache__` appears in `bench/`. The bwrap fork keeps its writes in a tmpfs inside its sandbox.
  The `arbor` section builds the prototype there, with a `CARGO_HOME` of its own so that cargo's lock and records of
  use stay out of `~/.cargo`, and gives arbor its state folder there (`ARBOR_HOME`), so it sees none of the person's
  worlds, keys, outbox or ledger. It also makes a small git repository there for `arbor look status`, with none of
  the person's git settings. The only files left behind are the reports in `results/`.
- **Reads, never writes:** `$HOME` (names only), the projects folder (text files for the code index, all files for
  hashing; files of 256 KiB and up are read with `O_DIRECT` so the scan does not flush the page cache), the
  transcripts (only aggregates leave the process), `/boot/initrd.img`, `/proc` and `/sys`. To build the prototype,
  cargo also reads the crates it has already downloaded and its config in `~/.cargo`, through links.
- **The prototype's service.** A cell in a cgroup of its own needs `arbor serve`, a transient systemd user service
  (`arbor.service` in `arbor.slice`). If it is not running, the section starts it and stops it again once its rows
  are read, also when the run is interrupted or times out, unless a cell that is not the section's runs in it by then.
  It never stops a service it did not start: if the service already runs another arbor binary, the two memory rows
  fail and their note says how to stop it.
- **The prototype's network.** To time connections through the proxy a key opens, the `arbor` section serves a short
  answer on this computer's network address, on a port the kernel picks, and answers only connections that come from
  this computer; it closes once they are timed. It listens there and not on `127.0.0.1` because a key never opens
  the loopback. Nothing goes beyond this computer. Where it has no network address, those two rows fail.
- **The screen.** The `screen` section reads the compositor's CPU time, context switches and GPU time from `/proc`,
  and asks GNOME's idle monitor over D-Bus how long ago a key or the mouse was last used; it never reads window
  titles or anything on the screen. It finds the compositor by connecting to the Wayland socket and asking who
  listens on it (`SO_PEERCRED`). For the first minute nothing should touch the PC: a minute with input fails the two
  still-screen rows. Then it opens a small window of its own, 480×270, which animates for about 21 s twice and
  closes. During the second run it starts one busy cell per CPU thread (`bwrap --unshare-all`, a shell loop) and a
  memory spike that takes all the memory available but 1 GiB, so for those seconds the PC is slow, other programs
  may be swapped out, and files leave the page cache. The cells, the spike and the window end with the section,
  also on a timeout or Ctrl-C. Leave it out with `SKIP=screen`.
- **The toolkits.** The `toolkits` section, only with `ONLY=toolkits`, builds the four test programs and
  `src/probe.c` into the temp dir with `toolkits/build.sh` (cargo with a `CARGO_HOME` of its own, as above), or uses
  the ones in `TOOLKITS_PREBUILT`. Cargo's objects, about
  a gigabyte a program in a temp dir that is in memory, are removed once each program is built, before anything is
  measured. Each program opens a window of its own, 960×640, several times: six short launches, a minute of still screen, and two animations of
  about 21 s, the second with the screen section's busy cells and memory spike. The programs keep their caches
  (Mesa's shaders, the toolkits' pipelines) in the temp dir. To time each frame, the probe is loaded into the program
  (`LD_PRELOAD`), asks the compositor when each new frame of its window was shown, and reads the program's CPU clock
  as it commits each frame; it reads nothing on the screen.
  The accessibility check starts the program once more in a D-Bus session of its own, where only the accessibility
  bus can be started and accessibility is turned on, with its settings in memory, a runtime folder in the temp dir
  (the accessibility bus would otherwise put its socket where the desktop's is) and no `DISPLAY`, so the desktop's
  accessibility bus and setting are never touched; it reads only that program's accessible objects. The windows,
  cells and spike end with the section, also on a timeout or Ctrl-C.
- **Tools:** Python 3.14+ (`compression.zstd`), node, gcc with static glibc (`libc6-dev`), rustc (optional,
  `~/.cargo/bin` is searched), rg, bwrap (0.11 or later for the overlay), unshare, docker (the user in the `docker`
  group, the image present), zstd, systemd-analyze, git, liblzo2 and liblz4 (for the heap compressors). The `arbor`
  section needs cargo with the toolchain that `../prototype/rust-toolchain.toml` pins and the prototype's crates
  already downloaded, since it builds offline and installs nothing (or `ARBOR_BIN`), and a systemd user manager for
  the cgroup and service rows. The `screen` section needs a Wayland session whose compositor has the
  presentation-time protocol (GNOME's has it), gcc with static glibc for its test client, which speaks the Wayland
  protocol itself and so needs no Wayland library, bwrap for the cells, and gdbus for the input check. The `toolkits`
  section needs the same, plus each toolkit's development files (`toolkits/README.md` lists them), libwayland-client,
  `dbus-run-session`, at-spi2-core and Python's `gi` with Atspi for the accessibility check. A missing tool fails
  only its rows, with the reason in the note.
- **User namespaces.** `cell.unshare_user`, `cell.unshare_4ns`, `storage.fork_ms` and `storage.fork_syscall_ms` run
  `unshare` from the suite's own process. `cell.bwrap`, `storage.fork_bwrap_ms` and `storage.fork_bwrap_overlay_ms`
  go through bwrap. Ubuntu 24.04 and later set `kernel.apparmor_restrict_unprivileged_userns = 1`: a process whose
  AppArmor profile does not allow user namespaces, such as a shell in an ordinary terminal, can still create one, but
  AppArmor confines it there to the `unprivileged_userns` profile, which denies the capabilities `unshare -Ur` needs.
  The suite checks this before timing: the four `unshare` rows then fail with a note that names the restriction, and
  the rest of their sections still runs. bwrap is not affected where AppArmor has a profile for it that allows user
  namespaces (`/etc/apparmor.d/bwrap-userns-restrict`, enabled by default from Ubuntu 25.04), so from an ordinary
  terminal the bwrap rows still measure a cell and a fork. Where bwrap has no such profile, its rows fail too, and
  their note says that this process may not use user namespaces. The M values of the `unshare` rows come from a run
  that could use them, so compare those rows only between runs with the same access. The suite never changes
  AppArmor.
- **Kernel settings it relies on:** Yama `ptrace_scope` at most 1 (the heap test reads its own child's memory).

## Sections

| Section | Measures | Method |
|---|---|---|
| `memstate` | swap and zram size, algorithm, use; RAM available; RSS of Claude Code, Electron, Chrome, GNOME | `/proc/swaps`, zram `mm_stat`, `/proc/meminfo`, `/proc/*/status`; taken first, before the suite loads the machine |
| `boot` | `systemd-analyze` totals, critical chain, blame; initramfs size by class; modules shipped vs loaded; modules on disk | the early uncompressed cpio archives are walked in place (newc headers), the main archive is piped through `zstd -dc`; module names compared with `/proc/modules` |
| `runtimes` | start time and idle RSS/PSS of static C, Rust, bash, python3, node (plus `/bin/true`, busybox, `python3 -I -S`) | `posix_spawn` + `wait4`, median of 40 runs after 3 warm-ups, in 3 interleaved blocks; RSS/PSS from `/proc/<pid>/smaps_rollup` 1.5 s after start (`ru_maxrss` under `posix_spawn` reports the parent) |
| `cells` | start of `unshare -Ur`, `unshare -Urnpfm`, `bwrap --unshare-all`, `docker run` | same timing method; docker: median of 8 |
| `pycell` | a Python cell importing 13 stdlib modules: fresh interpreter vs `fork()` of a warm template; PSS of each | fresh: median of 20; fork: median of 50; PSS over 10 live cells of each kind |
| `search` | name search over `$HOME`: `find -iname`, `rg --files -g`, and a catalog (one SQLite row per object with parent id and name, FTS5 trigram index on names); code search: `rg -l -F` vs an FTS5 trigram index | warm cache, median of 3 (index queries: median of 5); also catalog size per object, insert cost, build time, exact-name hash lookup |
| `heap` | per-4 KiB-page compressibility of an agent heap: zstd-3, zstd-1, LZO1X-1, LZ4, deflate-1; same-filled pages | node parses a 121 MiB transcript; its anonymous memory is read through `/proc/<pid>/mem` and each page compressed alone, as zram does. Read twice: with the file text still referenced (the state M caught) and after a memory-reducing GC (the live objects an idle agent holds) |
| `transcripts` | cost shares (input 1, cache write 1.25, cache read 0.1, output 5), opening context, context per call, step cost, read-only follow-up steps, tool-result tokens, errors, repeat reads, image tokens | one pass over every transcript, reproducing the four counting rules behind the M values (see the header of `sections/transcripts.js`); o200k tokens |
| `formats` | o200k tokens of one 100-file listing printed eight ways; `git log` with and without projection; `ps`, `ip`, `df` full vs only the asked fields | tokenizer in `lib/o200k.js`: the tiktoken regex and byte-pair merges over `o200k_base.tiktoken` |
| `storage` | duplicate bytes in the projects folder, counted once per inode and split by path class; time to fork the largest project with an overlay CoW mount, inside `unshare -Urm` and through bwrap; `git worktree add` as the baseline | `find -printf` for inodes and sizes, BLAKE2b over size-collision groups on all cores; mount timed as in M (`mount(8)` bracketed by `date`) and as the bare `mount(2)` syscall; through bwrap: `true` started in a sandbox with the overlay over the tree's own path (`--overlay-src` and `--tmp-overlay`), against the same sandbox without it, with the start-time method in 5 interleaved blocks; a write into either fork is checked not to reach the lower tree |
| `arbor` | the prototype, in a state folder of its own: making a world, starting a sealed command without and with a cgroup and under a key that opens a host, asking for an effect from inside a world and releasing one, a connection from inside a world through its proxy next to the same connection made directly, keeping and undoing a world that added 1,000 files, the idle memory of `arbor serve`, the size of the release binary, the o200k tokens of `arbor diff` for 100 changed files, and `arbor look`: its answers in o200k tokens (the 100-file listing, the 20 commits at `GIT_LOG_REF`, the 30 processes using the most memory, 15 changes in a git repository next to `git status` of them, and `arbor look` alone), one call that answers `status + log + files`, and a large file read in parts: the outline of each of this repository's ten largest code files, then two of its functions, as shares of the file's tokens | the start-time method in 3 interleaved blocks for the cell rows; asking is the difference between `arbor run W -- arbor effect -- true` and `arbor run W -- true`; `arbor new` median of 20, release median of 40, keep and undo median of 11 cycles; connections: 200 after 10 warm-ups in each of 3 interleaved blocks, each one opening, sending one request to a server on this computer's network address and reading its answer to the end (`lib/connect.py`); memory from `/proc/<pid>/smaps_rollup` of the service; each keep and undo is checked to move all 1,000 files, and `arbor diff` to list all 100; the `arbor look` answers are counted on the inputs of the `formats` section, each next to the usual command's output at the same moment (in its ctx), and the call is timed by the start-time method in 3 interleaved blocks, next to `git status`, `git log -20` and `ls -la` timed the same way; the files read in parts come from git at `OUTLINE_REF` into the temp dir, and each row is the median over them of the o200k tokens of `arbor look outline FILE`, then with `arbor look lines` of the two functions in the middle when the file's functions are put in order of length, over those of the whole file |
| `toolkits` | the same Mission Control test program in GTK 4, Qt Quick, Slint and iced: time to its first frame on screen, RSS and PSS after the start and after a minute of still screen, its CPU and wakeups on that still screen, frame pacing of its animation idle and under the screen section's load, the work of each frame, and its objects on the accessibility bus | first frame: from starting the program to the compositor's presentation time of the window's first frame, median of 5 launches after one left out; memory from `/proc/<pid>/smaps_rollup` over the program's processes; CPU and wakeups as in `screen`, over 60 s that start 8 s after the program did; frames: `src/probe.c`, loaded into the program, asks the compositor for the presentation time of each commit that brings the window a new buffer, and the screen section's maths counts the missed refreshes; work per frame: the program's CPU time, all its threads, from one commit of the window to the next, read by the probe at each commit, slowest 1%; accessibility: the program's objects walked with libatspi in a private D-Bus session, and whether its heading, card and rows' words are among them |
| `screen` | the compositor on a still screen: its CPU share and its wakeups per second over 60 s with no input; frame pacing of a test window: the share of refreshes that showed no new frame, and the slowest 1% of frame times, first on the machine as it is, then with a busy cell per CPU thread and a memory spike | the compositor is the process listening on the Wayland socket; CPU from `/proc/<pid>/stat` (all threads), wakeups as voluntary context switches summed over `/proc/<pid>/task/*/status`; input checked with GNOME's idle monitor; GPU time from `drm-engine-*` in `/proc/<pid>/fdinfo`. Frames: `src/frames.c`, a static Wayland client, draws a new frame on each `wl_surface.frame` for 1 s of warm-up and 20 s, and reports each frame's presentation (`wp_presentation`); missed refreshes are counted from the presentation sequence numbers, or from the timestamps where the compositor gives none |

## Reading the report

Each row shows the M value, the value now, the difference and a status:

- ✅ within ±15% of M, or within the precision M was printed with (0.1 MiB for the memory rows);
- ⚠️ outside ±15%, and the note says why;
- 🆕 a metric M did not have (first baseline);
- ❌ failed or not measured, with the error.

**†** marks an M value that comes from the M run's raw output rather than from the text of OPTIMIZATION.md (rows
next to the quoted ones, kept so they have a baseline too).

**‡** marks a reference that is not an M value: the row was added after the M investigation (the bwrap fork, the
prototype), and its reference comes from the suite's own run on the date in the report header (see `lib/reference.py`).
It is compared the same way.

The note on a ⚠️ row depends on what kind of number it is:

| Kind | Example | Why it can differ from M |
|---|---|---|
| timing on fixed inputs | start times | CPU load and frequency. The note gives the spread of the three block medians, the load average and the busiest processes at the start of the run, and says when M falls inside that spread |
| RSS | idle RSS of node | mostly file pages: a page fault also maps the cached neighbours of the page it needs, so RSS follows how much of the binary is in the page cache. The suite reads each binary first; the note gives the file/anonymous split |
| PSS | idle PSS of bash | shared pages are split among every process mapping them; the note counts the other processes running the same program. RSS is the stable floor |
| growing data set | `$HOME` entries, transcripts, projects folder | the note gives the count now vs at M, per-entry cost where it applies, and CPU load for timings. Code-search rows also say when `rg` had to read the corpus from disk |
| live state | swap in use, RSS of apps, the still screen, frame pacing | what runs right now; for the screen rows, also what other programs drew and the load beside the test window (the note says both) |
| boot records | `systemd-analyze` | a different boot than the one M described (the note gives both boot times), or a timer ran the unit again after M read `blame` |
| configuration | initramfs, modules | the file was rebuilt or packages changed |

For a fair before/after comparison: same PC, same CPU energy preference (the header records it), a quiet machine
(load average near 0; the header records it with the busiest processes), and a fresh boot if the boot rows matter.
This CPU mixes P- and E-cores, so under load a short single-thread timing may run on an E-core; runs at a load
average near 10 read 20–40% slower than M. Nothing is pinned to a core, so the prototype's rows timed on one arbor
process at a time (`arbor new`, keep, undo) move with where the scheduler puts it even on a quiet machine, and can
fall outside ±15% of their reference either way; their note says so. Join two `.json` files on `key` to diff runs.

Two M values needed a closer look:

- **Agent heap compressibility.** M's 2.15× (zstd) was read a second after loading, before V8's first major GC
  had freed the UTF-16 copy of the 121 MiB file text: that copy was about half of the 459 MiB read. The suite reproduces
  that state on purpose and also reads the heap after a memory-reducing GC, which is what an idle agent holds:
  about 186 MiB, compressing 1.78× with zstd-3 and 1.32× with LZO.
- **`rg` code search for `parseInt` (173.6 ms).** M's other four queries took 37–40 ms over the same files; with
  0.9 GiB RAM available, M's first query most likely read part of the corpus from disk. The suite gives each query
  an untimed warm-up run and records how much `rg` read from disk.

## Not covered

M numbers in OPTIMIZATION.md outside the seven areas this suite was asked to cover:

- the cost of a new TLS connection to the model API (12 ms of a 185 ms call): needs traffic to a third-party
  endpoint and depends on the network;
- pages identical across 4 agents for KSM (2%): needs several agents running at once;
- transcript dedup (−38%);
- outline-first code reading on the report's 40 files (4% and 8% of a file's tokens): the suite reads its own
  repository's largest files that way instead (`arbor.outline_pct`, `arbor.outline_read_pct`);
- the rebuild that follows `git worktree add` (1,774 steps);
- CPU facts read once (EPP on every core, sched_ext off, Meltdown/Spectre status): the report header records the
  energy preference of CPU 0.

Energy numbers in OPTIMIZATION.md come from the cited papers; reading RAPL counters on this kernel needs root.
