# The arbor prototype

The first working piece of arbor, built as a layer on today's Linux that needs no administrator rights. So far it has **worlds**: try any change on an instant copy of a folder, then keep the result or throw it away whole. It has **limits** on the memory, CPU, processes and time a command in a world may take. And it has **the ledger**: a record of every step and what it cost, checkable. Keys that seal a world further (M3), and the outbox and budgets (M6), come next. See the [roadmap](../README.md#where-it-stands).

## Try it

You need Linux with [bubblewrap](https://github.com/containers/bubblewrap) 0.10 or later, for its overlays, and Rust to build arbor. Ubuntu 26.04 ships bubblewrap 0.11 (`sudo apt install bubblewrap`). Ubuntu 24.04 ships 0.9, which has no overlays, so `arbor new` stops there and says so. Install the `arbor` command into `~/.cargo/bin`:

```sh
cargo install --locked --path prototype/arbor
```

Then, with any folder you work in:

```sh
arbor new shop --from ~/projects/shop   # an instant copy: nothing is copied until something changes
arbor shell shop                        # work inside it; the real folder does not change
arbor diff shop                         # what changed: A added, M modified, D deleted
arbor keep shop                         # apply the changes to the real folder
arbor undo shop                         # changed your mind? take them back out
arbor discard shop                      # throw the world away
```

`arbor run shop -- npm test` runs one command instead of a shell, and `arbor diff shop --patch` shows the changed lines. `arbor run shop --mem 2G --time 10m -- npm test` stops the tests if they take more than 2 GiB of memory or 10 minutes. `arbor worlds` lists every world, `arbor ps` what is running in them, and `arbor ledger` what happened.

## What a command inside a world can do

- **Change the folder, but only in the world.** The world is a copy-on-write layer mounted over the folder's own path, so tools see their usual paths. Every change lands in the world, and the real folder stays as it was until `arbor keep`.
- **Read the rest of the computer, not change it.** Everything else is read-only. Writes elsewhere in your home folder go to a throwaway layer that vanishes when the command ends.
- **No network.** It comes later, through a filter that lets in only the hosts you allow.
- **No way out through the computer's services.** `/run`, where Docker, D-Bus, systemd and your session keep their sockets, is empty inside a world, and so is arbor's own folder with the other worlds. A command cannot make user namespaces of its own, and it gets an empty keyring instead of your session's.

Not yet: files in your home folder, such as `~/.ssh`, the environment variables of the shell that started arbor, and hardware details under `/sys` can still be **read** from inside a world, and could be copied into it. Hiding them is the job of keys, next in M3. A world runs one command at a time.

**Read the diff before you keep.** Kept files such as `.git/hooks` or a project's scripts run later on your computer, outside any world.

## Keep, conflicts and undo

Before applying anything, `keep` checks that nothing it would overwrite or delete changed in the real folder after the world was made. If something did, it lists those paths and changes nothing. `keep --force` applies the world anyway.

Every file that keep replaces or deletes is moved into the world's undo folder first, so `arbor undo` can put the real folder back exactly as it was. Undo also moves the changes back into the world, so you can keep them again later. If you edited the real folder after keeping, undo refuses. `undo --force` then moves your later edits into the world instead of losing them.

Ctrl-C during a keep or an undo stops it between two changes, never halfway through one. After a stopped keep, `arbor undo` takes back what was applied; after a stopped undo, running it again finishes it. While a command runs in a world, keeping or undoing any world over the same folder waits for it: arbor says the world is busy.

## Limits

`arbor run` and `arbor shell` take limits before the command:

| Limit | When the command reaches it |
| :--- | :--- |
| `--mem 2G` | the whole command is stopped at once. Files it writes in `/tmp` count, and there is no swap to spill into |
| `--cpu 0.5` | it runs slower: never more than half of one CPU's time, or two CPUs' with `--cpu 2` |
| `--pids 64` | new processes are refused, so a runaway loop of them stops growing |
| `--time 10m` | the whole command is stopped |

Sizes take K, M, G or T (powers of 1024), and durations ms, s, m or h.

Each command in a world runs in a cgroup of its own, the Linux feature that counts and limits what a group of processes uses. Your systemd user session hands arbor that power without root, as it does on a current Ubuntu desktop. The first command starts `arbor serve`, a small user service that holds those cgroups and does nothing else; it stays until you log out, or until `systemctl --user stop arbor`, which also stops anything still running in a world. arbor makes each cell's cgroup itself, and the command starts inside it, so nothing sits between a command and its cell.

Where there is no systemd user session, as in some containers and CI, commands run without a cgroup: `--time` still works, while `--mem`, `--cpu` and `--pids` are refused rather than ignored. From an SSH login, arbor runs a command with those limits in a scope of your user session, and other commands without a cgroup. `ARBOR_NO_CGROUP=1` runs commands without a cgroup, to compare.

A command never outlives the arbor that started it: if arbor is killed, so is the command.

`arbor ps` lists the commands running in worlds now, with how long they have run, their CPU time and memory so far, and their limits.

## The ledger

Every step that changes something or runs a command is recorded: `new`, `run`, `shell`, `keep`, `undo` and `discard`, refusals and failures included. A command also records a `start` step when it begins. `arbor ledger` shows the last 20 (`--last N` for more), each with its time, result and cost. The cost is the wall time and, for a command, the CPU time of all its processes, the peak memory of the whole command and, where Linux keeps the count, how many processes it ran at once at most. A command that ran without a cgroup reports the peak memory of its largest process instead, and no process count. The result says when a limit stopped a command: `killed: memory limit`, `killed: time limit`, or `exit 2, process limit hit`.

Each line carries a SHA-256 hash over the line before it and itself, so `arbor ledger --verify` finds a line that was edited or deleted, and prints the head hash. A program running as you could still rewrite the whole file, so note the head hash if you may need to show later that nothing changed. Commands inside a world cannot see the ledger.

`arbor watch` prints each new step as a JSON line the moment it is recorded, without polling, for programs such as Mission Control. `arbor watch --since N` first prints the steps after step N, so a reader that missed some catches up by number.

## Where it keeps things

Worlds and the ledger live in `~/.local/share/arbor/` (`$XDG_DATA_HOME/arbor`, or `$ARBOR_HOME` if set). Each world holds only what it changed, plus, after a keep, the originals it replaced. Running commands' cgroups live under `arbor.slice` in your user session, and go when the command ends. To remove arbor:

1. Discard your worlds.
2. Run `systemctl --user stop arbor` and `cargo uninstall arbor`.
3. Delete that folder.

## Why bubblewrap

Ubuntu 24.04 and later stop ordinary programs from creating the Linux namespaces that sandboxes are built from, but they allow `bwrap`. A world's overlay is mounted inside a private `bwrap` namespace, so it exists only for the command running in it.

## For developers

| Path | What |
| :--- | :--- |
| `arbor/src/world.rs` | worlds on disk: names, `world.json`, the per-world lock |
| `arbor/src/tree.rs` | reading a world's changes (whiteouts, opaque folders), keep with its undo journal, and undo |
| `arbor/src/cell.rs` | running a command in a world through `bwrap`: its cgroup, time limit, how it ended and what it cost, and the list `arbor ps` reads |
| `arbor/src/cgroup.rs` | cells' cgroups: `arbor serve`, limits, what a cell used |
| `arbor/src/ledger.rs` | the ledger: appends under a lock, the hash chain, `--verify`, and following it for `watch` |
| `arbor/src/sys.rs` | the Linux calls the standard library lacks, such as `clone3` into a cgroup and inotify |
| `arbor/src/main.rs` | the `arbor` command |
| `arbor/tests/` | end-to-end tests through the binary and `bwrap`: `worlds.rs`, `cells.rs` (limits, `ps`, `watch`) |

`./check.sh` runs `cargo fmt --check`, `cargo clippy -D warnings` and `cargo test` with the toolchain pinned in `rust-toolchain.toml`. Tests that need `bwrap` print `skipped` and pass where it cannot mount an overlay, and tests of `--mem`, `--cpu` and `--pids` do the same where there is no systemd user session, so a green run there proves less. CI builds bubblewrap 0.11.1 from a pinned commit (its Ubuntu 24.04 runner ships 0.9), starts a user session for the runner, and sets `ARBOR_REQUIRE_BWRAP=1` and `ARBOR_REQUIRE_CGROUPS=1`, which turn those skips into failures.
