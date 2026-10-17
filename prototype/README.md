# The arbor prototype

The first working piece of arbor, built as a layer on today's Linux that needs no administrator rights. So far it has **worlds**: try any change on an instant copy of a folder, then keep the result or throw it away whole. And it has **the ledger**: a record of every step, checkable. Cells with keys and limits (M3), and the outbox and budgets (M6), come next. See the [roadmap](../README.md#where-it-stands).

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

`arbor run shop -- npm test` runs one command instead of a shell, and `arbor diff shop --patch` shows the changed lines. `arbor worlds` lists every world, and `arbor ledger` shows what happened.

## What a command inside a world can do

- **Change the folder, but only in the world.** The world is a copy-on-write layer mounted over the folder's own path, so tools see their usual paths. Every change lands in the world, and the real folder stays as it was until `arbor keep`.
- **Read the rest of the computer, not change it.** Everything else is read-only. Writes elsewhere in your home folder go to a throwaway layer that vanishes when the command ends.
- **No network.** It comes later, through a filter that lets in only the hosts you allow.
- **No way out through the computer's services.** `/run`, where Docker, D-Bus, systemd and your session keep their sockets, is empty inside a world, and so is arbor's own folder with the other worlds. A command cannot make user namespaces of its own, and it gets an empty keyring instead of your session's.

Not yet: files in your home folder, such as `~/.ssh`, the environment variables of the shell that started arbor, and hardware details under `/sys` can still be **read** from inside a world, and could be copied into it. Hiding them is the job of keys in M3, as are limits on time, CPU and memory. A world runs one command at a time.

**Read the diff before you keep.** Kept files such as `.git/hooks` or a project's scripts run later on your computer, outside any world.

## Keep, conflicts and undo

Before applying anything, `keep` checks that nothing it would overwrite or delete changed in the real folder after the world was made. If something did, it lists those paths and changes nothing. `keep --force` applies the world anyway.

Every file that keep replaces or deletes is moved into the world's undo folder first, so `arbor undo` can put the real folder back exactly as it was. Undo also moves the changes back into the world, so you can keep them again later. If you edited the real folder after keeping, undo refuses. `undo --force` then moves your later edits into the world instead of losing them.

Ctrl-C during a keep or an undo stops it between two changes, never halfway through one. After a stopped keep, `arbor undo` takes back what was applied; after a stopped undo, running it again finishes it. While a command runs in a world, keeping or undoing any world over the same folder waits for it: arbor says the world is busy.

## The ledger

Every step that changes something or runs a command is recorded: `new`, `run`, `shell`, `keep`, `undo` and `discard`, refusals and failures included. `arbor ledger` shows the last 20 (`--last N` for more), each with its time, result and cost. The cost is the wall time and, for a command, the CPU time of all its processes and the peak memory of the largest one.

Each line carries a SHA-256 hash over the line before it and itself, so `arbor ledger --verify` finds a line that was edited or deleted, and prints the head hash. A program running as you could still rewrite the whole file, so note the head hash if you may need to show later that nothing changed. Commands inside a world cannot see the ledger.

## Where it keeps things

Worlds and the ledger live in `~/.local/share/arbor/` (`$XDG_DATA_HOME/arbor`, or `$ARBOR_HOME` if set). Each world holds only what it changed, plus, after a keep, the originals it replaced. To remove arbor:

1. Discard your worlds.
2. Run `cargo uninstall arbor`.
3. Delete that folder.

## Why bubblewrap

Ubuntu 24.04 and later stop ordinary programs from creating the Linux namespaces that sandboxes are built from, but they allow `bwrap`. A world's overlay is mounted inside a private `bwrap` namespace, so it exists only for the command running in it.

## For developers

| Path | What |
| :--- | :--- |
| `arbor/src/world.rs` | worlds on disk: names, `world.json`, the per-world lock |
| `arbor/src/tree.rs` | reading a world's changes (whiteouts, opaque folders), keep with its undo journal, and undo |
| `arbor/src/cell.rs` | running a command in a world through `bwrap`, and what it cost |
| `arbor/src/ledger.rs` | the ledger: appends under a lock, the hash chain, `--verify` |
| `arbor/src/main.rs` | the `arbor` command |
| `arbor/tests/worlds.rs` | end-to-end tests through the binary and `bwrap` |

`./check.sh` runs `cargo fmt --check`, `cargo clippy -D warnings` and `cargo test` with the toolchain pinned in `rust-toolchain.toml`. Tests that need `bwrap` print `skipped` and pass where it cannot mount an overlay, so a green run there proves less. CI builds bubblewrap 0.11.1 from a pinned commit (its Ubuntu 24.04 runner ships 0.9) and sets `ARBOR_REQUIRE_BWRAP=1`, which turns a skip into a failure.
