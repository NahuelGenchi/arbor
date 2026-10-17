# Agent instructions

## The project

Arbor is a proposal for an operating system rebuilt for AI agents and the people who direct them: worlds (copy-on-write snapshots of everything an agent can see), cells (where work runs, started in a fraction of a millisecond), keys (narrow, expiring permissions), an outbox for effects that leave the machine, budgets, and a ledger of every step. Nothing of the operating system exists yet. What exists is the measurement: `OPTIMIZATION.md`, a report of where memory, disk, time, energy and agent tokens go on one Linux PC today, and `bench/`, a suite that re-measures the report's **M** numbers so a prototype can later be compared row by row. The first prototype is planned as a layer on top of Linux. `README.md` explains the project; the two articles behind it are at https://nahuelgenchi.com/software/arbor/ and https://nahuelgenchi.com/software/arbor-measured/.

Never describe something as working, built or measured unless a recorded check establishes it. A source check is not a benchmark run, and an estimate is not a measurement.

## Layout

| Path | Contents |
| :--- | :--- |
| `OPTIMIZATION.md` | The report of 16 October 2026: the waste measured today, the top 10 for the prototype, then RAM/CPU/GPU, storage and search, boot, speed, energy, agent tokens and network, what to remove, and what not to do. **M** = measured, otherwise a linked source, **est.** = estimate |
| `bench/run.sh` | Runs the suite and writes `bench/results/<date>_<host>.{md,json,log}` |
| `bench/config.sh` | Every path and input, each overridable from the environment |
| `bench/check.sh` | Source checks: every script parses |
| `bench/sections/` | One script per area: `memstate`, `boot`, `runtimes`, `cells`, `pycell`, `search`, `heap`, `transcripts` (JavaScript), `formats`, `storage` |
| `bench/lib/` | Shared helpers, the M reference values (`reference.py`), the report writer, the machine stamp, and an o200k token counter (`o200k.js`) |
| `bench/src/` | The static C and Rust programs timed by `runtimes` |
| `bench/results/` | Run reports, ignored by git |
| `docs/` | Tracking guide and README images |
| `docs/INTERFACE.md` | The Mission Control design brief: views, scale, motion, accessibility, targets and open questions. Its wireframes and their sources are in `docs/images/`. No interface exists yet |
| `.github/workflows/` | `checks.yml` runs `bench/check.sh` on every PR; `project-tracking.yml` keeps milestones and labels in order |

## Commands

- `bench/check.sh` runs the source checks (`bash -n`, `python3 -m py_compile`, `node --check`, `gcc -fsyntax-only`). It needs Python 3.14+, Node.js and gcc, reads no personal data and keeps compiled Python files out of `bench/`.
- `bench/run.sh` runs the whole suite, about 2½ minutes; `ONLY="boot search" bench/run.sh` or `SKIP="storage" bench/run.sh` for a subset. `bench/README.md` lists what it needs, reads and touches.

## Working well here

- Start from the active issue, then read only the code involved.
- The suite needs no root and must stay that way: namespaces are unprivileged, the overlay lives in a private mount namespace, temp files go in one `mktemp -d` folder removed on exit, and Docker runs with `--rm --pull never --network none`.
- It reads `$HOME` (names only), the projects folder, the transcripts and `/boot`, and never writes outside its temp folder and `bench/results/`. Only aggregates leave the transcript pass. Keep all of that true.
- A run on a busy machine reads slow: note the load average and the busiest processes, as the report header does, and compare runs only on the same PC under the same conditions (`bench/README.md`, *Reading the report*).
- A new number in `OPTIMIZATION.md` or the README is marked **M**, links its source, or says **est.**. A new M number gets a row in the suite, with its reference value in `bench/lib/reference.py`.

## Private data

A run's report names the machine, its hardware and the programs running on it, and the suite reads the owner's projects and transcripts. Hostnames, personal paths, transcript contents, project names that are not public, logs that contain them, and account credentials stay out of commits, issues and PRs. Quote only aggregate numbers.

## Tracking and delivery

Every change (code, bug fixes, enhancements, docs, research, measurements and repository operations) belongs to an issue with a milestone in `NahuelGenchi/arbor`, reused or created before editing. Every new feature, and every other work item, gets its own new branch created from the latest `origin/main` for that issue: `<type>/<issue>-<slug>` (`feat`, `fix`, `enh`, `docs`, `research`, `ops`). Never reuse a branch for a second feature, never mix two issues on one branch, and never push to `main`. Use `type: summary (#N)` commits. Delete the branch after its PR merges. PRs follow `.github/pull_request_template.md`: a standalone `Closes #N` line, the issue's milestone, and `Refs #N` for unfinished parent work. Decisions, exact changed paths and blockers go in the issue or PR. See `CONTRIBUTING.md` and `docs/TRACKING.md`.

## Final report

Report the issue, the PR and what changed. Keep these separate: source checks (exact commands and outcomes, or "none run"), benchmark runs (the same), rendering checks (the same), and source/diff review scope.
