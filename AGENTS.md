# Agent instructions

## The project

Arbor is a proposal for an operating system rebuilt for AI agents and the people who direct them: worlds (copy-on-write snapshots of everything an agent can see), cells (where work runs, started in a fraction of a millisecond), keys (narrow, expiring permissions), an outbox for effects that leave the machine, budgets, and a ledger of every step. The measurement came first: `OPTIMIZATION.md`, a report of where memory, disk, time, energy and agent tokens go on one Linux PC today, and `bench/`, a suite that re-measures the report's **M** numbers and measures the prototype the same way, so the two can be compared row by row. The first prototype, a rootless layer on top of Linux, is being built in `prototype/`: worlds (M2), sealed cells with limits and keys, which may open the network to the hosts they name (M3), the outbox, budgets and ledger (M6), and the first pieces of the agent layer (M4), `arbor look`, which answers an agent's reads as short tables, several in one call, down to a large file's outline and the lines it asks for, exist; the interface does not yet. `README.md` explains the project; the two articles behind it are at https://nahuelgenchi.com/software/arbor/ and https://nahuelgenchi.com/software/arbor-measured/.

Never describe something as working, built or measured unless a recorded check establishes it. A source check is not a benchmark run, and an estimate is not a measurement.

## Layout

| Path | Contents |
| :--- | :--- |
| `OPTIMIZATION.md` | The report of 16 October 2026: the waste measured today, the top 10 for the prototype, then RAM/CPU/GPU, storage and search, boot, speed, energy, agent tokens and network, what to remove, and what not to do. **M** = measured, otherwise a linked source, **est.** = estimate |
| `bench/run.sh` | Runs the suite and writes `bench/results/<date>_<host>.{md,json,log}` |
| `bench/config.sh` | Every path and input, each overridable from the environment |
| `bench/check.sh` | Source checks: every script parses |
| `bench/sections/` | One script per area: `memstate`, `boot`, `runtimes`, `cells`, `pycell`, `search`, `heap`, `transcripts` (JavaScript), `formats`, `storage`, `arbor`, which builds the prototype into its temp folder and measures it, and `screen`, the still-screen cost of the compositor and the frame pacing of a test window |
| `bench/lib/` | Shared helpers, the M reference values (`reference.py`), the report writer, the machine stamp, and an o200k token counter (`o200k.js`) |
| `bench/src/` | The static C and Rust programs timed by `runtimes`, and `frames.c`, the static Wayland test client of `screen` |
| `bench/sessions/` | The A/B of whole agent sessions, apart from the suite: `ab.py` runs OpenCode on its free model in fresh worlds, on the tasks in `tasks.py`, with and without `arbor look` in the world's `AGENTS.md`, and counts each session from OpenCode's own records; `summary.py` writes the report |
| `bench/results/` | Run reports, ignored by git |
| `prototype/` | The first prototype in Rust: one binary, `arbor` (`arbor/src/`: `world`, `tree` for diff/keep/undo, `cell` for sealed `bwrap` cells, `seccomp` for the kernel calls a cell may not make, `cgroup` for limits, `key` for keys and their home folders, `net` for the hosts a key opens and the proxy that reaches them, `outbox` for effects and budgets, `ledger`, `look` for the reads an agent makes, answered as tables, `outline` for the definitions in a source file and the lines each spans), end-to-end tests in `arbor/tests/`, and `check.sh`. The toolchain is pinned in `rust-toolchain.toml` |
| `docs/` | Tracking guide and README images |
| `docs/INTERFACE.md` | The Mission Control design brief: views, scale, motion, accessibility, targets and open questions. Its wireframes and their sources are in `docs/images/`. No interface exists yet |
| `.github/workflows/` | `checks.yml` runs `bench/check.sh` and `prototype/check.sh` on every PR; `project-tracking.yml` keeps milestones and labels in order |

## Commands

- `bench/check.sh` runs the source checks (`bash -n`, `python3 -m py_compile`, `node --check`, `gcc -fsyntax-only`). It needs Python 3.14+, Node.js and gcc, reads no personal data and keeps compiled Python files out of `bench/`.
- `bench/run.sh` runs the whole suite, about 2½ minutes plus 2 for `screen`, which needs the PC left alone; `ONLY="boot search" bench/run.sh` or `SKIP="storage" bench/run.sh` for a subset. `bench/README.md` lists what it needs, reads and touches.
- `python3 bench/sessions/ab.py` runs the A/B of whole sessions, 5 per task and arm (`--reps N`); it needs OpenCode in `~/.opencode/bin` and its network, and writes `bench/results/sessions_<date>_<host>.{jsonl,md}`. `bench/README.md`, *Whole agent sessions*, says how.
- `prototype/check.sh` runs `cargo fmt --check`, `cargo clippy -D warnings` and `cargo test`. The end-to-end tests need `bwrap` 0.10+, and the limit tests a systemd user manager; each skips with a note where it is missing. CI builds bubblewrap 0.11.1, starts a user manager, and sets `ARBOR_REQUIRE_BWRAP=1` and `ARBOR_REQUIRE_CGROUPS=1`, so there they must run. `cargo build --release` in `prototype/` builds `target/release/arbor`; `prototype/README.md` shows how to use it.

## Working well here

- Start from the active issue, then read only the code involved.
- The suite needs no root and must stay that way: namespaces are unprivileged, the overlay lives in a private mount namespace, temp files go in one `mktemp -d` folder removed on exit, and Docker runs with `--rm --pull never --network none`.
- The prototype needs no root either. On Ubuntu 24.04+ AppArmor blocks plain `unshare` for ordinary programs but allows `bwrap`, so cells go through `bwrap`; check new sandbox code from an ordinary process (`systemd-run --user --pipe --wait …`), not only from an agent session, whose AppArmor profile may allow more. `arbor` writes only its state folder (`$ARBOR_HOME`, else `~/.local/share/arbor`, where a cell under a key made with `--home` also writes, in that key's home folder), on keep or undo the world's base folder, and its own cgroups under `arbor.slice` in the user's systemd manager (held by `arbor serve`, a transient user service); `arbor release` also runs, as the user, an effect's command that a person approved, and the `arbor run` or `arbor shell` of a cell whose key opens hosts connects to those hosts, over HTTPS, for the cell. Tests set `ARBOR_HOME` and `HOME` to temp folders.
- It reads `$HOME` (names only), the projects folder, the transcripts, `/boot` and, to build the prototype offline, the crates and config in `~/.cargo`, and never writes outside its temp folder and `bench/results/`: cargo gets a `CARGO_HOME` in the temp folder. The `arbor` section starts `arbor serve` when it is not running and stops it again, also when the run is interrupted, and while it times connections through a cell's proxy it serves on this computer's network address, answering only this computer. The `screen` section reads only process and thread names and timings, never window titles or screen contents; its test window, cells and memory spike end with it, also when the run is interrupted. Only aggregates leave the transcript pass. `bench/sessions/ab.py` runs OpenCode only on its free model, with no login and settings only its keys' homes see, reaches only OpenCode's two hosts, and reads from OpenCode's records only the session tables, never the one with logins. Keep all of that true.
- A run on a busy machine reads slow: note the load average and the busiest processes, as the report header does, and compare runs only on the same PC under the same conditions (`bench/README.md`, *Reading the report*).
- A new number in `OPTIMIZATION.md` or the README is marked **M**, links its source, or says **est.**. A new M number gets a row in the suite, with its reference value in `bench/lib/reference.py`.

## Private data

A run's report names the machine, its hardware and the programs running on it, and the suite reads the owner's projects and transcripts. Hostnames, personal paths, transcript contents, project names that are not public, logs that contain them, and account credentials stay out of commits, issues and PRs. Quote only aggregate numbers.

## Tracking and delivery

Every change (code, bug fixes, enhancements, docs, research, measurements and repository operations) belongs to an issue with a milestone in `NahuelGenchi/arbor`, reused or created before editing. Every new feature, and every other work item, gets its own new branch created from the latest `origin/main` for that issue: `<type>/<issue>-<slug>` (`feat`, `fix`, `enh`, `docs`, `research`, `ops`). Never reuse a branch for a second feature, never mix two issues on one branch, and never push to `main`. Use `type: summary (#N)` commits. Delete the branch after its PR merges. PRs follow `.github/pull_request_template.md`: a standalone `Closes #N` line, the issue's milestone, and `Refs #N` for unfinished parent work. Decisions, exact changed paths and blockers go in the issue or PR. See `CONTRIBUTING.md` and `docs/TRACKING.md`.

## Final report

Report the issue, the PR and what changed. Keep these separate: source checks (exact commands and outcomes, or "none run"), benchmark runs (the same), rendering checks (the same), and source/diff review scope.
