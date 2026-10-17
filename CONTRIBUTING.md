# Contributing

Keep the work small, traceable, and honest about what has been measured and what has not. Agents must also follow [AGENTS.md](AGENTS.md).

## One work item, one delivery trail

1. **Start with an issue.** Reuse the issue that already owns the work, or open the work-item form. Record the goal, scope, completion criteria, dependencies, and milestone before editing files. Bugs, enhancements, documentation, research and repository operations are work items too.
2. **Create a new branch for the issue.** Every new feature, and every other work item, is developed on its own new branch, created from the latest `main`:

   ```sh
   git fetch origin
   git switch -c feat/7-energy-section origin/main
   ```

   Name it `<type>/<issue>-<slug>`. Types are `feat`, `fix`, `enh`, `docs`, `research` and `ops`. One branch holds one issue: never reuse a merged or existing branch for a new feature, and never mix two features on one branch. Use focused commits such as `feat: measure package power in the energy section (#7)`. Do not commit directly to `main`. Delete the branch after its PR merges.
3. **Open a linked PR.** Include a standalone `Closes #N` line for each completed, same-repository issue. Set the PR to the same milestone as its primary issue. Split cross-milestone changes into separate PRs. Use `Refs #N` for unfinished parent or related work; it does not replace a closing reference to the scoped issue.
4. **Record the outcome.** Describe exactly what changed, which checks ran, and what was not run. Close the issue only when its stated scope is complete.

For non-file operations (repository settings, releases), put sanitized commands and results in the issue. When an operation produces no code change, deliver its lasting decision through a small documentation PR instead of an empty PR.

## What belongs where

**Issues** own scope, decisions, blockers, dependencies, and outcomes. **Milestones** group outcomes. **PRs** own the proposed diff and delivery record. **Documentation** owns reusable knowledge, not duplicate task status.

Every meaningful action must stay discoverable through that chain. Routine reads do not need separate issues. Split genuinely new scope into a linked issue instead of expanding a task indefinitely.

## Checks

`bench/check.sh` runs the source checks: `bash -n` on the shell scripts, `python3 -m py_compile` on every Python file, `node --check` on the JavaScript and `gcc -fsyntax-only` on the C program. They establish that the suite parses, nothing more, and the Checks workflow runs them on every PR. A **benchmark run** (`bench/run.sh`) is the real check of a change to the suite: name the machine, the sections run and the load average at the start, and quote the report's result line. Report each kind separately in the PR's delivery record, and say what was not run.

A number that goes into `OPTIMIZATION.md` or the README is marked **M** only when it was measured, links to its source otherwise, and says **est.** when it is an estimate.

## Private data

A run of the suite reads the projects folder, `$HOME` and the agent transcripts, and its report names the machine, its hardware and the programs running on it. Reports stay in `bench/results/`, which git ignores. Keep hostnames, personal paths, transcript contents, logs that contain them and account credentials out of commits, issues, and PRs; quote only aggregate numbers. The ignore rules are an accidental-staging guard, not permission to collect that data. If a secret is exposed, stop sharing it and revoke it.

## Documentation standard

Write clear English. Prefer specific paths and actionable instructions. Distinguish **implemented**, **planned**, **blocked**, and **unknown**: arbor is a proposal with measurements until a prototype exists. Do not invent commands, measurements, or compatibility. Update documentation in the same PR as the change it describes. Keep `README.md` approachable for people who do not write code, and `AGENTS.md` limited to what agents need in every session.

## Tracking automation is not branch protection

The metadata workflow assigns missing milestones and labels PRs that need tracking. It does not run checks or block pushes or merges. See [docs/TRACKING.md](docs/TRACKING.md).
