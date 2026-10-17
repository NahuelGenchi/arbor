# The arbor prototype

The first working piece of arbor, built as a layer on today's Linux that needs no administrator rights. So far it has **worlds**: try any change on an instant copy of a folder, then keep the result or throw it away whole. A command in a world runs **sealed**: it sees the world and little else of yours, until a **key** opens more, for a while. It has **limits** on the memory, CPU, processes and time a command may take. And it has **the ledger**: a record of every step and what it cost, checkable. The outbox and budgets (M6) come next. See the [roadmap](../README.md#where-it-stands).

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

`arbor run shop -- npm test` runs one command instead of a shell, and `arbor diff shop --patch` shows the changed lines. Inside a world your home folder is empty, so tools installed there, such as `~/.nvm` or `~/.cargo`, need a key: `arbor key new dev --toolchains` makes one that opens them, and `arbor run shop --key dev -- npm test` uses it. `arbor run shop --mem 2G --time 10m -- npm test` stops the tests if they take more than 2 GiB of memory or 10 minutes. `arbor worlds` lists every world, `arbor ps` what is running in them, `arbor stop` stops one, and `arbor ledger` shows what happened.

## What a command inside a world can do

- **Change the folder, but only in the world.** The world is a copy-on-write layer mounted over the folder's own path, so tools see their usual paths. Every change lands in the world, and the real folder stays as it was until `arbor keep`.
- **See little else of yours.** Inside a world your home folder is empty, so are `/media` and `/mnt`, and the other folders in `/home` are gone. Anything written there vanishes when the command ends. The rest of the computer, such as its programs in `/usr`, can be read but not changed. Only a few plain variables of the shell that started arbor pass in: `PATH`, `HOME`, `USER`, `LOGNAME`, `SHELL`, `TERM`, `COLORTERM`, `NO_COLOR`, `LANG`, `LANGUAGE`, `LC_*`, `TZ`, `EDITOR`, `VISUAL` and `PAGER`. The computer's name reads `arbor`.
- **Open more only with a key**, for a while: see [Keys](#keys). Password files stay hidden even then.
- **No network.** It comes later, through a filter that lets in only the hosts you allow.
- **No way out through the computer's services.** `/run`, where Docker, D-Bus, systemd and your session keep their sockets, is empty inside a world, and so is arbor's own folder with the other worlds and the keys.
- **Fewer ways into the kernel.** A filter turns away the requests to the Linux kernel that real work rarely needs and that escapes from a sandbox usually start with: the kernel's keyrings, where some logins keep secrets; mounting, and namespaces of a command's own; BPF, perf and io_uring; and a few more. io_uring and the newer mount calls are reported as missing, which tells programs to use the older calls. The filter also keeps a command from typing into your terminal, so a shell in a world keeps job control (Ctrl-Z, `fg`). Debuggers and `strace` still work on the world's own programs. 32-bit programs cannot run in a world: they stop at their first request, with exit code 159 (*Bad system call*).

Not yet: the rest of the computer can still be **read** from inside a world, such as `/etc`, the hardware details under `/sys`, and folders outside `/home`, `/media` and `/mnt`. A world runs one command at a time.

**Read the diff before you keep.** Kept files such as `.git/hooks` or a project's scripts run later on your computer, outside any world.

## Keys

A key opens a little more than the world, for one job and until a set time, like the valet key of a car. `arbor key new NAME` makes one, with any of:

- `--read PATH`: a folder or file of yours that the command may read. Writes to a folder vanish when the command ends, so tools that keep caches there still work. Repeat it to open more.
- `--toolchains`: the language tools in your home folder, those that exist of `~/.cargo` and `~/.rustup`, `~/.nvm`, `~/.npm`, `~/.bun`, `~/.deno`, `~/.volta` and `~/.local/share/pnpm`, `~/.pyenv`, `~/.local/bin`, `~/.local/lib` and `~/.cache/pip`, `~/go`, `~/.rbenv` and `~/.gem`, and `~/.sdkman`, `~/.gradle` and `~/.m2`.
- `--env VAR`: passes that variable in from the shell that runs the command, such as `CARGO_HOME`. Repeat it to pass more.
- `--world W`: opens only that world; without it, any. Repeat it to open more.
- `--mem`, `--cpu`, `--pids`, `--time`: the [limits](#limits) below. A command run with the key gets the tighter of the key's limits and its own.
- `--for 2h` or `--until 18:00`: when the key ends. Without either, it lasts 8 hours.

`arbor run shop --key dev -- npm test` and `arbor shell shop --key dev` use it. A key that has ended, or that does not open the world, is refused, and a command still running when its key ends is stopped (`killed: key expired`). `arbor keys` lists the keys: what each opens, its limits, and when it ends, as a clock time.

`arbor key new tests --within dev --read ~/.cargo` makes a narrower key from another, for a smaller job. It may open only what `dev` opens, with limits no larger and an end no later, and whatever it leaves out it takes from `dev`. `arbor key take-back dev` ends `dev` and every key made within it, and stops the commands running under them (`stopped: key taken back`).

A key cannot open the whole computer, the folders a cell gets fresh copies of (`/proc`, `/dev`, `/sys` and `/run`), or arbor's own folder. Each key is a small file in arbor's folder that only you can read, and it holds names and paths, never a password or a token.

**Password files stay hidden** inside a world, even when the world or a key includes the folder they are in. Only a world or a key that is that file or folder itself shows it. A hidden folder looks empty, a hidden file looks empty, and writes to them vanish. These are the ones in your home folder:

- keys and passwords: `.ssh`, `.gnupg`, `.password-store`, `.local/share/keyrings` and `.pki`;
- cloud and service logins: `.aws`, `.azure`, `.config/gcloud`, `.kube`, `.docker/config.json`, `.config/gh`, `.git-credentials`, `.netrc`, `.vault-token` and `.terraform.d/credentials.tfrc.json`;
- package registry tokens: `.npmrc`, `.pypirc`, `.gem/credentials`, `.cargo/credentials`, `.cargo/credentials.toml`, `.gradle/gradle.properties`, `.m2/settings.xml` and `.m2/settings-security.xml`;
- browser and mail profiles, with saved passwords and signed-in sessions: `.mozilla`, `snap/firefox`, `.thunderbird`, `.config/google-chrome`, `.config/chromium`, `.config/BraveSoftware` and `.config/microsoft-edge`.

## Keep, conflicts and undo

Before applying anything, `keep` checks that nothing it would overwrite or delete changed in the real folder after the world was made. If something did, it lists those paths and changes nothing. `keep --force` applies the world anyway.

Every file that keep replaces or deletes is moved into the world's undo folder first, so `arbor undo` can put the real folder back exactly as it was. Undo also moves the changes back into the world, so you can keep them again later. If you edited the real folder after keeping, undo refuses. `undo --force` then moves your later edits into the world instead of losing them.

Ctrl-C during a keep or an undo stops it between two changes, never halfway through one. After a stopped keep, `arbor undo` takes back what was applied; after a stopped undo, running it again finishes it. While a command runs in a world, keeping or undoing any world over the same folder waits for it: arbor says the world is busy.

## Limits

`arbor run` and `arbor shell` take limits before the command. When the command reaches one:

- `--mem 2G`: the whole command is stopped at once. Files it writes in `/tmp` count, and there is no swap to spill into.
- `--cpu 0.5`: it runs slower, never taking more than half of one CPU's time (`2` would allow two CPUs).
- `--pids 64`: new processes are refused, so a runaway loop of them stops growing.
- `--time 10m`: the whole command is stopped.

Sizes take K, M, G or T (powers of 1024), and durations ms, s, m or h.

Each command in a world runs in a cgroup of its own, the Linux feature that counts and limits what a group of processes uses. Your systemd user session hands arbor that power without root, as it does on a current Ubuntu desktop. The first command starts `arbor serve`, a small user service that holds those cgroups and does nothing else; it stays until you log out, or until `systemctl --user stop arbor`, which also stops anything still running in a world. arbor makes each cell's cgroup itself, and the command starts inside it, so nothing sits between a command and its cell.

Where there is no systemd user session, as in some containers and CI, commands run without a cgroup: `--time` still works, while `--mem`, `--cpu` and `--pids` are refused rather than ignored. From an SSH login, arbor runs a command with those limits in a scope of your user session, and other commands without a cgroup. `ARBOR_NO_CGROUP=1` runs commands without a cgroup, to compare.

`arbor stop CELL` stops a command running in a world, and so does closing the terminal it runs in, or `kill` on the arbor that runs it: arbor stops the command and records `stopped`. A command never outlives the arbor that started it: if arbor is killed outright (`kill -9`), so is the command.

`arbor ps` lists the commands running in worlds now, with their cell, world and key, how long they have run, their CPU time and memory so far, and their limits.

## The ledger

Every step that changes something or runs a command is recorded: `new`, `run`, `shell`, `stop`, `keep`, `undo`, `discard`, `key` (making one) and `take-back`, refusals and failures included. A command also records a `start` step when it begins. `arbor ledger` shows the last 20 (`--last N` for more), each with its time, key, result and cost. The cost is the wall time and, for a command, the CPU time of all its processes, the peak memory of the whole command and, where Linux keeps the count, how many processes it ran at once at most. A command that ran without a cgroup reports the peak memory of its largest process instead, and no process count. The result says when a limit, a key or a person stopped a command: `killed: memory limit`, `killed: time limit`, `exit 2, process limit hit`, `killed: key expired`, `stopped: key taken back` or `stopped`.

Each line carries a SHA-256 hash over the line before it and itself, so `arbor ledger --verify` finds a line that was edited or deleted, and prints the head hash. A program running as you could still rewrite the whole file, so note the head hash if you may need to show later that nothing changed. Commands inside a world cannot see the ledger.

`arbor watch` prints each new step as a JSON line the moment it is recorded, without polling, for programs such as Mission Control. `arbor watch --since N` first prints the steps after step N, so a reader that missed some catches up by number.

## Where it keeps things

Worlds, keys and the ledger live in `~/.local/share/arbor/` (`$XDG_DATA_HOME/arbor`, or `$ARBOR_HOME` if set). Each world holds only what it changed, plus, after a keep, the originals it replaced. Running commands' cgroups live under `arbor.slice` in your user session, and go when the command ends. To remove arbor:

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
| `arbor/src/cell.rs` | running a command in a world through `bwrap`: what a sealed cell sees, its environment, its cgroup, deadlines and `stop`, how it ended and what it cost, and the list `arbor ps` reads |
| `arbor/src/seccomp.rs` | the seccomp filter every cell runs under: the kernel calls it refuses, as a classic BPF program that bwrap loads |
| `arbor/src/key.rs` | keys: what they open, narrowing one within another, their end, take-back, and the password files a cell never sees |
| `arbor/src/cgroup.rs` | cells' cgroups: `arbor serve`, limits, what a cell used |
| `arbor/src/ledger.rs` | the ledger: appends under a lock, the hash chain, `--verify`, and following it for `watch` |
| `arbor/src/sys.rs` | the Linux calls the standard library lacks, such as `clone3` into a cgroup and inotify |
| `arbor/src/main.rs` | the `arbor` command |
| `arbor/tests/` | end-to-end tests through the binary and `bwrap`, each with its own throwaway home folder: `worlds.rs` (worlds, what a sealed cell sees, the kernel calls it refuses), `cells.rs` (limits, `ps`, `watch`), `keys.rs` (what a key opens, password files, `stop`, take-back) |

`./check.sh` runs `cargo fmt --check`, `cargo clippy -D warnings` and `cargo test` with the toolchain pinned in `rust-toolchain.toml`. Tests that need `bwrap` print `skipped` and pass where it cannot mount an overlay, and tests of `--mem`, `--cpu` and `--pids` do the same where there is no systemd user session, so a green run there proves less. CI builds bubblewrap 0.11.1 from a pinned commit (its Ubuntu 24.04 runner ships 0.9), starts a user session for the runner, and sets `ARBOR_REQUIRE_BWRAP=1` and `ARBOR_REQUIRE_CGROUPS=1`, which turn those skips into failures.
