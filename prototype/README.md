# The arbor prototype

The first working piece of arbor, built as a layer on today's Linux that needs no administrator rights. So far it has **worlds**: try any change on an instant copy of a folder, then keep the result or throw it away whole. A command in a world runs **sealed**: it sees the world and little else of yours, and no network, until a **key** opens more, for a while, such as the hosts it may reach. It has **limits** on the memory, CPU, processes and time a command may take. What a command in a world needs done outside it, such as a `git push`, waits in **the outbox** until you release it, which runs it once, or refuse it, and a key can cap what those **effects** cost. And it has **the ledger**: a record of every step and what it cost, checkable. For agents, **`arbor look`** answers the reads they make most as short tables, several in one call. See the [roadmap](../README.md#where-it-stands).

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

`arbor run shop -- npm test` runs one command instead of a shell, and `arbor diff shop --patch` shows the changed lines. Inside a world your home folder is empty, so tools installed there, such as `~/.nvm` or `~/.cargo`, need a key: `arbor key new dev --toolchains` makes one that opens them, and `arbor run shop --key dev -- npm test` uses it. When a command, or the program a script runs with, is only in a place the world hides, arbor says so before it starts, and names the key that opens it:

```console
$ arbor run shop -- cargo test
arbor: `cargo` is in ~/.cargo/bin, which the world hides. A key opens it: `arbor key new dev --toolchains`, then add `--key dev`.
```

`arbor run shop --mem 2G --time 10m -- npm test` stops the tests if they take more than 2 GiB of memory or 10 minutes. `arbor worlds` lists every world, `arbor ps` what is running in them, `arbor stop` stops one, and `arbor ledger` shows what happened. A command in a world asks for what only you can do, such as `arbor effect -- git push`, and `arbor outbox` shows what waits for your answer: see [The outbox](#the-outbox).

## What a command inside a world can do

- **Change the folder, but only in the world.** The world is a copy-on-write layer mounted over the folder's own path, so tools see their usual paths. Every change lands in the world, and the real folder stays as it was until `arbor keep`.
- **See little else of yours.** Inside a world your home folder is empty, so are `/media` and `/mnt`, and the other folders in `/home` are gone. Anything written there vanishes when the command ends, unless a key gives the command a [home folder of its own](#a-keys-own-home-folder). The rest of the computer, such as its programs in `/usr`, can be read but not changed. Only a few plain variables of the shell that started arbor pass in: `PATH`, `HOME`, `USER`, `LOGNAME`, `SHELL`, `TERM`, `COLORTERM`, `NO_COLOR`, `LANG`, `LANGUAGE`, `LC_*`, `TZ`, `EDITOR`, `VISUAL` and `PAGER`. The computer's name reads `arbor`.
- **Open more only with a key**, for a while: see [Keys](#keys). Password files, and what AI agents keep of your sessions, stay hidden even then.
- **No network, unless a key opens hosts.** A world has a network of its own with nothing in it. A key made with `--net` opens the hosts it names, over HTTPS only: see [The network](#the-network). What needs more can wait for you in [the outbox](#the-outbox).
- **No way out through the computer's services.** `/run`, where Docker, D-Bus, systemd and your session keep their sockets, holds only `/run/arbor` inside a world: the socket that asks the outbox, and the `arbor` command that asks through it. Arbor's own folder, with the other worlds, the keys, the outbox and the ledger, is empty.
- **Ask for an effect**, and only ask: `arbor effect -- CMD` asks for `CMD` to run outside the world once you say yes. It is one of the two arbor commands that work inside a world; the other, [`arbor look`](#reads-for-an-agent), only reads.
- **Fewer ways into the kernel.** A filter turns away the requests to the Linux kernel that real work rarely needs and that escapes from a sandbox usually start with: the kernel's keyrings, where some logins keep secrets; mounting, and namespaces of a command's own; BPF, perf and io_uring; and a few more. io_uring and the newer mount calls are reported as missing, which tells programs to use the older calls. The filter also keeps a command from typing into your terminal, so a shell in a world keeps job control (Ctrl-Z, `fg`). Debuggers and `strace` still work on the world's own programs. 32-bit programs cannot run in a world: they stop at their first request, with exit code 159 (*Bad system call*).

Not yet: the rest of the computer can still be **read** from inside a world, such as `/etc`, the hardware details under `/sys`, and folders outside `/home`, `/media` and `/mnt`. A world runs one command at a time.

**Read the diff before you keep.** Kept files such as `.git/hooks` or a project's scripts run later on your computer, outside any world.

## Keys

A key opens a little more than the world, for one job and until a set time, like the valet key of a car. `arbor key new NAME` makes one, with any of:

- `--read PATH`: a folder or file of yours that the command may read. Writes to a folder vanish when the command ends, so tools that keep caches there still work. Repeat it to open more.
- `--toolchains`: the language tools in your home folder, those that exist of `~/.cargo` and `~/.rustup`, `~/.nvm`, `~/.npm`, `~/.npm-global` (where npm puts the tools it installs without root), `~/.bun`, `~/.deno`, `~/.volta` and `~/.local/share/pnpm`, `~/.pyenv`, `~/.local/bin`, `~/.local/lib` and `~/.cache/pip`, `~/go`, `~/.rbenv` and `~/.gem`, and `~/.sdkman`, `~/.gradle` and `~/.m2`.
- `--env VAR`: passes that variable in from the shell that runs the command, such as `CARGO_HOME`. Repeat it to pass more.
- `--net HOST`: a host the command may reach over HTTPS, such as `api.example.com`: see [The network](#the-network). Repeat it to open more.
- `--home`: a home folder of the key's own, in place of the empty one, kept from one run to the next: see [A key's own home folder](#a-keys-own-home-folder).
- `--world W`: opens only that world; without it, any. Repeat it to open more.
- `--mem`, `--cpu`, `--pids`, `--time`: the [limits](#limits) below. A command run with the key gets the tighter of the key's limits and its own.
- `--spend 5`: what the effects released under the key may cost in all, here $5.00 (cents as `2.50`): see [The outbox](#the-outbox).
- `--for 2h` or `--until 18:00`: when the key ends. Without either, it lasts 8 hours.

`arbor run shop --key dev -- npm test` and `arbor shell shop --key dev` use it. A key that has ended, or that does not open the world, is refused, and a command still running when its key ends is stopped (`killed: key expired`). `arbor keys` lists the keys: what each opens, its limits, and when it ends, as a clock time.

`arbor key new tests --within dev --read ~/.cargo` makes a narrower key from another, for a smaller job. It may open only what `dev` opens, with limits and a spend no larger and an end no later, and whatever it leaves out it takes from `dev`, its home folder included. `arbor key take-back dev` ends `dev` and every key made within it, stops the commands running under them (`stopped: key taken back`), and refuses the effects asked for under them that still wait.

`arbor key renew dev` lets a key last longer, whether it has ended or not: 8 hours from now, or `--for 2h` or `--until 18:00`. Only its end changes, and only to a later one:

- **Everything else stays**: what the key opens, its limits, what its effects have spent of its `--spend`, and its home folder with what is in it.
- **A command running under it runs on** until the new end, as if the key had been made to last that long.
- **Never sooner.** An end that is not later than the key's end now is refused. To end a key sooner, take it back.
- **Keys made within it keep their ends.** Each can be renewed on its own, up to the end of the key it was made within, which is also where it goes without `--for` or `--until`. While that key has ended, it cannot be renewed.

`arbor keys` shows the new end, and the ledger records each renewal: `key dev until 20:00 instead of 18:00`.

A key cannot open the whole computer, the folders a cell gets fresh copies of (`/proc`, `/dev`, `/sys` and `/run`), arbor's own folder, or this computer's own services on `localhost`. Each key is a small file in arbor's folder that only you can read, and it holds names and paths, never a password or a token.

**Password files stay hidden** inside a world, even when the world or a key includes the folder they are in. Only a world or a key that is that file or folder itself shows it. A hidden folder looks empty, a hidden file looks empty, and writes to them vanish. They stay hidden from a command that is already running when their program replaces one, by renaming a new file over it or by deleting it and writing it again, also where it is linked from your home folder to elsewhere on the computer. One that is missing when a command starts stays missing for that command, even when a program makes it while the command runs, as when you sign in to an agent meanwhile. What the command writes there itself is its own, as anywhere else: thrown away in a folder a key opens, and among a world's changes in a world. These are the ones in your home folder:

- keys and passwords: `.ssh`, `.gnupg`, `.password-store`, `.local/share/keyrings` and `.pki`;
- cloud and service logins: `.aws`, `.azure`, `.config/gcloud`, `.kube`, `.docker/config.json`, `.config/gh`, `.git-credentials`, `.netrc`, `.vault-token` and `.terraform.d/credentials.tfrc.json`;
- package registry tokens: `.npmrc`, `.pypirc`, `.gem/credentials`, `.cargo/credentials`, `.cargo/credentials.toml`, `.gradle/gradle.properties`, `.m2/settings.xml` and `.m2/settings-security.xml`;
- browser and mail profiles, with saved passwords and signed-in sessions: `.mozilla`, `snap/firefox`, `.thunderbird`, `.config/google-chrome`, `.config/chromium`, `.config/BraveSoftware` and `.config/microsoft-edge`;
- AI agents' logins:
  - OpenCode's whole folder, `.local/share/opencode`; Claude Code's `.claude/.credentials.json`, `.claude.json`, which holds its sign-in session, with its copies in `.claude/backups`, and `.config/anthropic/credentials` for a Console login made without an API key; Codex's `.codex/auth.json`, `.codex/.credentials.json` for its logins to MCP servers, and `.codex/secrets`, where it keeps them encrypted when their key is in the keyring;
  - more whole folders: Gemini CLI's `.gemini`, Qwen Code's `.qwen`, Copilot's in editors, `.config/github-copilot`, Amp's `.local/share/amp`, Goose's `.config/goose`, Crush's `.local/share/crush`, Factory's Droid's `.factory`, Cline's `.cline/data`, Kilo's `.local/share/kilo`, OpenHands' `.openhands`, Amazon Q's `.local/share/amazon-q`, and the Cursor editor's `.config/Cursor`;
  - the Copilot CLI's `.copilot/config.json`, `.copilot/mcp-oauth-config` and `.copilot/mcp-secrets`; Cursor's CLI's `.config/cursor/auth.json` and `.cursor/projects`; Kimi Code's `.kimi-code/credentials` and `.kimi-code/config.toml`, and Kimi CLI's `.kimi/credentials` and `.kimi/config.toml`; Aider's `.aider/oauth-keys.env` and `.aider.conf.yml`; Auggie's `.augment/session.json`; and Pi's `.pi/agent/auth.json`;
  - API keys in `.env`, which Gemini CLI, Qwen Code and Aider read, in Mistral Vibe's `.vibe/.env` and in Continue's `.continue/.env`.

OpenCode keeps its logins in its database, `opencode.db`, next to every session, and writes the latest changes to files beside it, which come and go as it runs. So the whole folder stays hidden, with OpenCode's old login file and its MCP servers' tokens. A key that names one file in it, such as `--read ~/.local/share/opencode/opencode.db`, opens that file alone, and OpenCode in a world gets a login of its own as a copy: see [OpenCode, with the login it already has](#opencode-with-the-login-it-already-has).

The other whole folders are hidden for the same reasons. Some programs keep their logins in a database, or in files whose names change from one version to the next. Others, Gemini CLI's at every refresh, replace a login by renaming a new file over it, which they first write under another name that no list names, and a hidden folder hides that one too. Their settings still open with a key that names them, such as `--read ~/.gemini/settings.json`. A key that names the folder itself, such as `--read ~/.gemini`, opens the logins in it, but not the browser profile that Gemini CLI's browser agent signs in to sites with, `.gemini/cli-browser-profile`. The list names the places programs use by default: a login moved elsewhere, as with `CLAUDE_CONFIG_DIR`, `CODEX_HOME` or `GEMINI_CLI_HOME`, is not hidden.

**Agents' records of your sessions stay hidden** in the same way. A transcript holds each message, each file the agent read and each command's output, including a password a tool printed. An agent also keeps the prompts you typed, notes from past sessions, and copies of files and pastes, for every project. Hidden, they keep a world for one project from seeing your work on the others. What a broad grant such as `--read ~/.claude` is for still shows: Claude Code's settings, `CLAUDE.md`, skills, agents and plugins; Codex's `config.toml`, `AGENTS.md`, skills and prompts; Gemini CLI's settings, `GEMINI.md`, commands and skills; or the Copilot CLI's settings, instructions, agents and skills. These are the ones in your home folder:

- Claude Code's transcripts, with what Claude notes for each project, `.claude/projects`; every prompt you typed, `.claude/history.jsonl`; files as they were before it changed them, `.claude/file-history`; pastes and attachments, `.claude/paste-cache`, `.claude/image-cache` and `.claude/uploads`; plans and task lists, `.claude/plans`, `.claude/tasks` and `.claude/todos`; debug logs, `.claude/debug`; transcripts packed for feedback, and drafts, `.claude/feedback-bundles` and `.claude/feedback`; `/insights` reports, `.claude/usage-data`; background sessions, `.claude/jobs`; and subagents' notes, `.claude/agent-memory`;
- Codex's transcripts, `.codex/sessions` and `.codex/archived_sessions`; the messages you sent, `.codex/history.jsonl`; sessions' names, `.codex/session_index.jsonl`; logs, `.codex/log`; memories of past chats, `.codex/memories`, `.codex/memories_v2` and `.codex/memories_extensions`; and its databases, with the threads' history and the queue of your messages, `.codex/*.sqlite*`, that is, every name with `.sqlite` in it, whatever version it carries, with the files SQLite keeps beside each;
- OpenCode's prompt history, stashed prompts and recently opened files, `.local/state/opencode`. Its sessions are in `.local/share/opencode`, hidden with its logins;
- Gemini CLI's, for each project, `.gemini/tmp`: transcripts, the prompts you typed, shell-mode commands, chats saved with `/chat save`, checkpoints, plans, long tool outputs and its notes on the project; and the snapshots of your projects behind its checkpoints, `.gemini/history`. Its whole folder is hidden with its logins, and these stay hidden even from a key that opens it by name, as Claude Code's do from `--read ~/.claude`;
- the Copilot CLI's sessions, with each one's events, plan, reports and copies of files before an edit, `.copilot/session-state`; the database of all sessions, `.copilot/session-store.db*`, with the files SQLite keeps beside it; your prompts, `.copilot/command-history-state*`, a folder, or a file in earlier versions; logs, `.copilot/logs`; and sessions from versions before 1.0, `.copilot/history-session-state`;
- Aider's chat, `.aider.chat.history.md`; the lines you typed, `.aider.input.history`; and, when turned on, the messages to and from the model, `.aider.llm.history`. Aider keeps them in the project, at the top of its git repository, so these are the ones from Aider run in the home folder. Those in a project come next.

**An agent's database is missing instead.** SQLite cannot work with a database hidden as a file: it reads empty and the first write fails. In a world, the journal SQLite leaves beside it would also reach your database through `arbor keep` and empty it. So Codex's and the Copilot CLI's databases, with the files SQLite keeps beside each, look missing, in a folder that throws away what a command writes there. The agent makes a new database, which lasts until the command ends:

- **In a folder a key opens**, such as `--read ~/.codex`, writes vanish already, and everything else there shows as before.
- **In a world of a folder that holds `~/.codex` or `~/.copilot`**, such as a world of your home folder, that folder throws away what a command writes in it, settings included. `arbor diff` and `arbor keep` never see it, and your databases stay as they were. What the world changed in that folder before does not show there.
- **Under a key with a home folder of its own**, the agent's database there stays for its next run.

**Records in a project's folder stay hidden too.** Some agents keep their records in the project rather than in your home folder:
- Aider's three files above, at the top of the project's git repository, or in the folder it started in when there is none;
- Crush's `.crush`, with its database of every session's messages and copies of the files it edited, its logs and the output of long commands.

A world over a project, or a key that opens one, hides them where these agents would have written them:

- at the top of the world's folder and of each folder a key opens;
- in each folder right inside one of those, so a key that opens a folder of projects, such as `--read ~/work`, hides each project's;
- in the folder the command starts in, and in each folder above it, up to the top of the world.

Deeper ones are not looked for, since searching a whole project would take longer than starting the command. In a world of your home folder, `~/work/shop`'s are hidden only from a command that starts in `~/work/shop` or below.

`.crush` is hidden whole, since SQLite cannot open a database hidden as a file. The project's settings and commands for Crush are hidden with it.

In a world, what is hidden is what the folder held before the world changed it:
- **What was there.** A hidden file reads empty and writes to it vanish. `arbor diff` and `arbor keep` never see it, so the real file stays as it was.
- **What a command writes** where there was none is the world's own, like any file it writes. `arbor diff` shows it, `arbor keep` applies it, and the world's next commands see it, so Aider and Crush in a world find what they wrote there on their next run.

A key that names one, such as `--read ~/work/shop/.crush`, opens it. Not hidden:
- a record made in a project while a command runs;
- records moved elsewhere, as with Aider's `--chat-history-file` or Crush's `--data-dir`.

**A key that names one opens it on purpose.** `--read ~/.claude/projects/-home-you-work-shop` opens one project's sessions, and its `memory` folder opens Claude's notes on that project. Under a key that opens only `~/.claude`, an agent keeps your settings but finds no past session to resume. Under a key with a home folder of its own, what the agent keeps there stays for its next run, as its logins do. As with the password files, the list names the places programs use by default. Records moved with `CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `CODEX_SQLITE_HOME`, `XDG_STATE_HOME`, `GEMINI_CLI_HOME` or `COPILOT_HOME`, or with an option such as Aider's `--chat-history-file`, are not hidden. What arbor hides is decided when a command starts, and what is missing then stays missing for it, including the files SQLite keeps beside each database there, which it writes when Codex or Copilot starts after the command. A `*` stands only for the names there when the command starts: a database of a new version, made while it runs, is not hidden from it.

### A key's own home folder

A command in a world starts with an empty home folder, and what it writes there vanishes when it ends. A program that keeps its settings there, as an agent's program does, starts as if for the first time on every run. A key made with `--home` gives the command a home folder of the key's own instead, kept from one run to the next:

```sh
arbor key new notes --home
arbor run shop --key notes -- sh -c 'echo dark > ~/.theme'
arbor run shop --key notes -- cat ~/.theme   # dark
```

- **Only commands under the key see it**, at your home folder's usual path. Under another key, or none, the home folder is empty as before, and your real home folder is never touched.
- **The world, and what the key opens, lie on top of it** as they lie on top of the empty one: a change in the world stays in the world, and writes to what the key opens still vanish.
- **It lives in arbor's folder**, in `~/.local/share/arbor/homes/NAME`, which only you can read, and outside every world, so `arbor diff` and `arbor keep` leave it out. `arbor keys` shows what it takes on disk. Nothing caps its size, as nothing caps a world's changes.
- **Password files and agents' records.** What a command writes there, such as a login or a transcript of its own, stays visible to the next command under the key: the folder is for that. Yours stay hidden as before, since the key's home takes your home folder's place.
- **It lives as long as the key**, and [renewing the key](#keys) makes both last longer. `arbor key take-back` removes it, once the key's commands have stopped. A key that has ended keeps it until it is taken back, but opens nothing until `arbor key renew` moves its end later; its commands then find the folder as they left it. A key made later under the same name starts with an empty one; if a folder of that name is left over, as when a key's file was deleted by hand, arbor refuses to make the key until the folder is moved away or removed. If the folder itself is deleted, the next command under the key gets a new, empty one.
- **A key made `--within` one with a home folder shares it**, unless it says `--home`, which gives it an empty one of its own. `--home` within a key that has no home folder is refused.
- **Every world the key opens shares it.** What a command leaves there, such as its settings or a script the program runs when it starts, reaches the next command under the key in any of them. Make the key with `--world W` to keep its home folder to one world.
- **A key with a home folder cannot also open yours**, or a folder that holds it, and its commands cannot run in a world of your home folder: in both, its home folder would take that place.

## The network

A world has a network of its own with nothing in it, so a command there reaches no other computer and none of the services on yours. A key made with `--net` opens the hosts it names, and nothing else:

```sh
arbor key new deps --toolchains --net registry.npmjs.org
arbor run shop --key deps -- npm install
```

`--net` takes a host name such as `api.example.com`; `*.example.com` for every name below `example.com`, but not `example.com` itself; `HOST:PORT` for a port other than 443; or an IP address. A pasted `https://` address counts as its host.

- **Only HTTPS goes out, and only through arbor.** The command's first process is arbor itself: it opens one port on the world's own loopback, `127.0.0.1:3128`, hands it to the `arbor run` or `arbor shell` that started the command, and becomes the command. That arbor answers the port as a proxy until the command ends: it checks each host against the key, connects from outside the world, and passes the bytes both ways. They stay encrypted, so it never reads them. `HTTPS_PROXY` and its kin point tools there, and most tools follow them: curl, git, npm, pip and Node.js programs, for which arbor sets `NODE_USE_ENV_PROXY=1`. A tool that ignores them reaches nothing. Plain HTTP is refused, and so are SSH, other protocols and name lookups from inside the world.
- **A host name never leads back to this computer or its network.** A name that resolves to a loopback, private, link-local or similar address is refused, so a name in a key cannot reach your router, a service on your network, or a cloud server's metadata address. A key opens such an address only by naming it, as in `--net 192.168.1.20:8443`. `localhost` and the loopback addresses cannot be opened at all: inside a world they are the world's own.
- **You see what a key kept out.** When the command ends, arbor names the hosts the key did not open:

  ```console
  arbor: key 'deps' does not open github.com, so the command could not reach it. A key opens a host with `--net HOST`.
  ```

- **The ledger records it.** Each refused host is a `net` step the first time a command asks for it, so `arbor watch` shows it at once, and when the command ends a `net` step sums up what it reached: the connections to each host, and the bytes in and out.

A key made `--within` another opens only hosts the other opens; one that names none takes them all.

### An agent in a world

An agent's program runs in a world like any other command. Its key opens the folder its program is installed in, passes its API key in by name from your shell, opens its API's host, and gives it a home folder of its own for its settings. For an agent installed in your home folder, such as Claude Code:

```sh
arbor key new agent --toolchains --read ~/.local/share/claude --env ANTHROPIC_API_KEY --net api.anthropic.com --home
arbor shell shop --key agent
```

Inside, the agent changes only the world, reaches only its API, and asks for anything else through [the outbox](#the-outbox); `arbor diff shop` shows what it changed. An agent may try other hosts too, such as its maker's telemetry: arbor names each one it refused, so you can decide whether to open it. What it keeps in its home folder, such as Claude Code's `~/.claude` and `~/.claude.json`, is still there the next time it runs under the key, in [the key's own home folder](#a-keys-own-home-folder), and goes when the key is taken back. When the key ends, `arbor key renew agent` lets it last another 8 hours, and the agent starts with the settings it left. Every world the key opens shares that folder, so a key for each project, made with `--world`, keeps them apart.

#### OpenCode, with the login it already has

OpenCode keeps the login of a plan, such as OpenCode Go, in its database, `~/.local/share/opencode/opencode.db`, next to every session it has had. A key cannot open that login alone, so it goes into the key's own home folder as a copy, once:

```sh
arbor key new opencode --read ~/.opencode/bin --home --net opencode.ai --net models.opencode.ai
opencode auth export opencode-go | arbor run shop --key opencode -- opencode auth import --standalone
arbor run shop --key opencode -- opencode models --standalone
arbor run shop --key opencode -- opencode run --standalone -m opencode/deepseek-v4.1-flash-free "say hi"
```

- **The key** opens OpenCode's program, which its installer puts in `~/.opencode/bin`, and two hosts: `opencode.ai`, its models' API, for a plan's models and for its free ones, and `models.opencode.ai`, its list of models. OpenCode asked for no other host.
- **The copy.** `opencode auth export opencode-go` prints the plan's login, its secret included, and `opencode auth import`, in the world, stores it in the key's home folder. The pipe keeps it off the disk on the way, and naming `opencode-go` copies that login alone: without it, every login OpenCode has would go in. Only commands under the key see the copy, and taking the key back deletes it. Opening the login with `--read` does not work: OpenCode 2 reads its old login file, `~/.local/share/opencode/auth.json`, only while it upgrades a database from OpenCode 1, and the key's home starts with a new one.
- **Its list of models.** In a new home folder, OpenCode knows only the models it was built with until it has fetched the current list, in the background, and kept it there. Until then, `opencode run -m` with a newer model ends with `Model unavailable`. `opencode models` lists the ones it knows; if yours is not there yet, run it again.
- **`--standalone`** runs OpenCode's server inside the command. Without it, OpenCode starts its background service in the world, which works too and ends with the command.
- **Its settings** are those in the key's home folder, never yours. The checks below spent nothing, background calls such as session titles included, with this `~/.config/opencode/opencode.json` there, put in with `arbor run shop --key opencode -- sh -c 'mkdir -p ~/.config/opencode && cat > ~/.config/opencode/opencode.json' < opencode.json`. `enabled_providers` allows only OpenCode's own models, whose free ones need no login, so not even the plan's login can be used: leave it out to work on the plan.

  ```json
  {
    "model": "opencode/deepseek-v4.1-flash-free",
    "small_model": "opencode/deepseek-v4.1-flash-free",
    "enabled_providers": ["opencode"],
    "autoupdate": false
  }
  ```

Checked on 17 October 2026 with OpenCode 2.0.29, on its free model only:

- In a world, OpenCode read a file and changed it, with `--auto`, which lets it make changes without asking: they land in the world. `arbor diff shop` showed that change and nothing else, and the real folder kept its old version until `arbor keep`.
- A later run under the key went on with the same session (`opencode run -c`), also once the key had ended and `arbor key renew` had brought it back.
- With an `AGENTS.md` in the world saying so, OpenCode read the file it was asked about with `arbor look outline` and `arbor look lines`.
- The plan's own login, copied by its owner with the pipe above, was in the world: `opencode auth list` there named OpenCode Go as stored. OpenCode then answered on its free model, with every other provider turned off by the settings above. No call was made on the plan's models, which would have used its quota, and taking the key back deleted the copy.

## Reads for an agent

At every step an agent's model reads its whole conversation again, so a read that takes one step instead of three, and prints only what was asked, saves twice. `arbor look` answers the reads an agent makes most as short tables, several in one call:

```console
$ arbor look status + log -n 3 + files src
# status: on main, 1 ahead of origin/main
path	change
notes.txt	untracked
src/cart.rs	modified
# log -n 3: first 3, and more
hash	subject
4f1c2ab	fix: round the total once
9e07d13	feat: a cart that keeps its items
51b8a6e	docs: how to run the shop
# files src
name
cart.rs
lib.rs
pay/
```

It answers six queries:

- `files [DIR]`: what a folder holds, by name; a folder's name ends in `/`.
- `status`: what changed in the git repository, by path from its top folder.
- `log [REV] [PATH]...`: commits, newest first, of a branch or a range, or those that changed a path.
- `procs [TEXT]`: processes, the ones that use the most memory first; with TEXT, those whose command holds it.
- `outline FILE`: the definitions in a source file, in order, and the lines each spans.
- `lines FILE [A[-B]]`: a file's lines A to B, numbered.

Each gives a few fields unless asked for others: `-f name,size,modified`, or `-f all`. `-n` asks for more or fewer rows, and `-n 0` for all of them. `arbor look` alone describes every query and field, so an agent needs no tool description in its prompt, only to know that `arbor look` is there. How the answers are written:

- **A title line first.** Each answer starts with `# QUERY`, which says what was cut (`first 100 of 2345`, `lines 1-200 of 2193`) and, for `status`, the branch, and for `outline`, how many lines the file has.
- **Failures.** A query that fails says why on its title line, and the others still answer; arbor then exits 1. A mistake in how a query is asked stops them all before any runs.
- **Tabs between the columns**, which cost fewer tokens than aligned columns.
- **One unit each:** sizes and memory are bytes, times `YYYY-MM-DDTHH:MM` in local time, modes octal.
- **Characters spelled out.** Characters a terminal would act on, such as a tab or an escape in a file name, are spelled out (`\t`, `\e`).

It only reads:

- **git writes nothing.** `status` and `log` run git so that it writes nothing, not even its index, and no word of a query reaches git as an option.
- **No ledger step,** as for `arbor worlds`.
- **Inside a world it sees what the world sees:** the world's folder, and the world's own processes.

### A large file in parts

An agent that reads a whole file to change one function pays for all of it, and again at every step after. `outline` gives the file's definitions, each with the lines it spans, from the comments, attributes and decorators above it to its end, so the agent reads only the ones it needs with `lines`:

```console
$ arbor look outline src/cart.rs
# outline src/cart.rs: 37 lines
line	end	kind	name
5	11	struct	Item
13	17	struct	Cart
19	37	impl	Cart
20	22	fn	new
24	30	fn	add
32	36	fn	total
$ arbor look lines src/cart.rs 32-36
# lines src/cart.rs 32-36: lines 32-36 of 37
line	text
32	    /// The total, rounded once at the end.
33	    pub fn total(&self) -> Price {
34	        let sum: f64 = self.items.iter().map(|i| i.price.0 * f64::from(i.count)).sum();
35	        Price((sum * 100.0).round() / 100.0)
36	    }
```

`-f all` adds each definition's `parent`, the `impl`, class, module or heading it sits in, and its `signature`, its head on one line. On this repository's ten largest code files, the outline is a median **4.2%** of a file's tokens, and the outline with two of its functions **8.5%** (**M**: the suite's `arbor.outline_pct` and `arbor.outline_read_pct`).

`outline` reads Rust, Python, JavaScript, TypeScript and shell files, Markdown files' headings, and scripts whose `#!` line names python, node or a shell; another kind of file is an error on its title line, and so is a binary one, with a NUL byte in its first 8,000 bytes, as git tells them. Each language is read line by line, with no parser: comments and strings are blanked out, definitions are found by their keywords at the start of a line, and where each ends by its braces (Rust, JavaScript, TypeScript, shell), its indentation (Python) or the next heading of its level (Markdown). Reading that way misses or gets wrong:

- definitions made by macros, written after other code on their line, or inside a function's body, an object or a call (a test suite's `describe`, a route's handler); in Rust, the functions of an `extern` block;
- in JavaScript and TypeScript, a `/` that could start a regular expression or divide, and a quote in JSX text, which can end a definition early or late; and a function called where it is written, as compilers write classes (`var A = function () {…}()`), which is listed as a function;
- in Python, an f-string that holds its own quotes;
- in shell, a function whose body is in parentheses;
- in Markdown, headings inside HTML, and all but the last line of a setext heading's text.

A shorter answer is not yet a cheaper session: an agent told to read this way may take more steps to reach the same lines. [`bench/README.md`](../bench/README.md#whole-agent-sessions), *Whole agent sessions*, measures both over whole sessions of OpenCode, with and without `arbor look`.

## The outbox

A command in a world reaches no network but the hosts its key opens, and nothing it does reaches your real files until you keep the world. When the work needs something done outside, such as pushing a branch, sending an email or paying for a service, the command asks for it:

```sh
arbor effect --cost 0.40 --why "publish the fix" -- git push origin fix
```

That **effect** waits in the outbox, and the command goes on. With `--wait` the command waits for your answer instead, and prints what the effect printed. `--input FILE`, or `--input -` for what the command pipes in, gives the effect up to 1 MiB to read. Outside the world, you answer:

```sh
arbor outbox                     # what waits for you
arbor outbox 3                   # all about effect 3: what it runs and where, its cost, who asked and why
arbor release 3                  # run it now: as you, in the real folder, with the network
arbor refuse 3 --why "not yet"   # say no
```

`arbor release` runs the command in the real folder, at the same place the command in the world asked from, with your rights and your environment. It shows what the command prints, and ends with the command's exit code. An effect runs once at most: it is marked running before its command starts, so a second release is refused, and one whose arbor was killed while it ran shows as `unknown` and never runs again. Its receipt stays with it: the exit code, how long it took, and the size, SHA-256 and first 1 MiB of what it printed. Throwing a world away refuses the effects it asked for that still wait, and so does taking back the key they were asked under.

The cost and the reason are the asker's words: arbor shows them but cannot check them. A key made with `--spend 5` caps them. Under it, an effect must say what it costs, and `arbor release` refuses one that would take the key, or a key it was made within, past what it may spend. `arbor keys` shows what each key has spent.

**Read the command before you release it**, since it runs as you. arbor shows it as a shell would read it, with any character that could act on your terminal spelled out, such as `\e` or `\n`, so what you read is what runs.

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

Every step that changes something or runs a command is recorded: `new`, `run`, `shell`, `stop`, `keep`, `undo`, `discard`, `key` (making one), `renew` and `take-back`, for effects `effect` (a command in a world asked for one), `release` and `refuse`, and `net` for [the network](#the-network) a command used, refusals and failures included. A command also records a `start` step when it begins, and a release records itself before its command starts, so the ledger has it even if arbor is killed. `arbor ledger` shows the last 20 (`--last N` for more), each with its time, key, result and cost. The cost is the wall time and, for a command, the CPU time of all its processes, the peak memory of the whole command and, where Linux keeps the count, how many processes it ran at once at most. A command that ran without a cgroup reports the peak memory of its largest process instead, and no process count. A released effect's cost also has the money it said it would cost. The result says when a limit, a key or a person stopped a command: `killed: memory limit`, `killed: time limit`, `exit 2, process limit hit`, `killed: key expired`, `stopped: key taken back` or `stopped`. A command the world cannot find never starts: its result is `not found`, and arbor exits with 127, as a shell does.

Each line carries a SHA-256 hash over the line before it and itself, so `arbor ledger --verify` finds a line that was edited or deleted, and prints the head hash. A program running as you could still rewrite the whole file, so note the head hash if you may need to show later that nothing changed. Commands inside a world cannot see the ledger.

`arbor watch` prints each new step as a JSON line the moment it is recorded, without polling, for programs such as Mission Control. `arbor watch --since N` first prints the steps after step N, so a reader that missed some catches up by number.

## Where it keeps things

Worlds, keys and their home folders, the outbox and the ledger live in `~/.local/share/arbor/` (`$XDG_DATA_HOME/arbor`, or `$ARBOR_HOME` if set). Each world holds only what it changed, plus, after a keep, the originals it replaced. A key made with `--home` has its home folder in `homes`. The outbox keeps each effect with its input and the start of what it printed. Running commands' cgroups live under `arbor.slice` in your user session, and go when the command ends. To remove arbor:

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
| `arbor/src/key.rs` | keys: what they open, narrowing one within another, their end and its renewal, take-back, their own home folders, and the password files and agents' records a cell never sees |
| `arbor/src/cgroup.rs` | cells' cgroups: `arbor serve`, limits, what a cell used |
| `arbor/src/outbox.rs` | the outbox: effects on disk, the socket a cell asks through and the threads that answer it, release (once, with a receipt), refusal, and budgets |
| `arbor/src/ledger.rs` | the ledger: appends under a lock, the hash chain, `--verify`, and following it for `watch` |
| `arbor/src/table.rs` | the short tables arbor prints, and text from a world shown so that it cannot act on the terminal |
| `arbor/src/look.rs` | `arbor look`: its queries and their fields, the tab-separated answers, git run as a reader, processes read from `/proc`, and a file read in parts |
| `arbor/src/outline.rs` | the definitions in a source file and the lines each spans, for `arbor look outline`: a reader for each language, line by line, with no parser |
| `arbor/src/net.rs` | the network a key opens: hosts and how they match, the port a cell's first process opens and hands out, and the proxy that answers it (`CONNECT` only), with the tally the ledger records |
| `arbor/src/sys.rs` | the Linux calls the standard library lacks, such as `clone3` into a cgroup, inotify and `poll`, passing a descriptor over a socket, and threads that leave signals to the main one |
| `arbor/src/main.rs` | the `arbor` command |
| `arbor/tests/` | end-to-end tests through the binary and `bwrap`, each with its own throwaway home folder: `worlds.rs` (worlds, what a sealed cell sees, the kernel calls it refuses), `cells.rs` (limits, `ps`, `watch`), `keys.rs` (what a key opens, password files and agents' records, a key's own home folder, `stop`, renewal, take-back), `outbox.rs` (asking, release once with a receipt, refusal, budgets, `--wait`, what a world's words can do to a terminal), `net.rs` (the hosts a key opens, reached through the proxy from a cell, and what it refuses), `look.rs` (reads answered as tables, on a throwaway folder and repository, a file's outline and lines, and inside a world) |

`./check.sh` runs `cargo fmt --check`, `cargo clippy -D warnings` and `cargo test` with the toolchain pinned in `rust-toolchain.toml`. Tests that need `bwrap` print `skipped` and pass where it cannot mount an overlay, and tests of `--mem`, `--cpu` and `--pids` do the same where there is no systemd user session, so a green run there proves less. CI builds bubblewrap 0.11.1 from a pinned commit (its Ubuntu 24.04 runner ships 0.9), starts a user session for the runner, and sets `ARBOR_REQUIRE_BWRAP=1` and `ARBOR_REQUIRE_CGROUPS=1`, which turn those skips into failures.
