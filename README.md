<div align="center">
  <img src="docs/images/logo.svg" width="96" height="96" alt="arbor logo: a trunk that splits into three branches, two ending in hollow rings and the middle one in a gold dot">
  <h1>arbor</h1>
  <p><strong>An operating system built for AI agents, and for the people who direct them.</strong></p>
  <p>
    Every computer you have used was built for one person at a keyboard.<br>
    Agents are now some of the busiest users our machines have.<br>
    arbor is a plan to rebuild the computer around them, measured on a real PC first, and its first piece runs on today's Linux.
  </p>
  <p>
    <img src="https://img.shields.io/badge/status-proposal%2C%20measured%2C%20first%20prototype-9f6811?style=flat-square" alt="Status: a proposal, measured, with a first prototype">
    <img src="https://img.shields.io/badge/measured-16%20Oct%202026-1b8355?style=flat-square" alt="Measured on 16 October 2026">
    <img src="https://img.shields.io/badge/first%20step-a%20layer%20on%20Linux-4b4841?style=flat-square&logo=linux&logoColor=white" alt="First step: a layer on top of Linux">
    <a href="https://github.com/NahuelGenchi/arbor/actions/workflows/checks.yml"><img src="https://img.shields.io/github/actions/workflow/status/NahuelGenchi/arbor/checks.yml?branch=main&style=flat-square&label=checks&color=4b4841" alt="Checks on main"></a>
    <a href="https://nahuelgenchi.com/software/arbor/"><img src="https://img.shields.io/badge/read-the%20proposal-7a5fd0?style=flat-square" alt="Read the proposal"></a>
  </p>
  <p>
    <a href="#in-one-minute">In one minute</a> &nbsp;·&nbsp;
    <a href="#what-you-get">What you get</a> &nbsp;·&nbsp;
    <a href="#one-task-start-to-finish">One task</a> &nbsp;·&nbsp;
    <a href="#measured-not-guessed">Measured</a> &nbsp;·&nbsp;
    <a href="#try-the-prototype">Try it</a> &nbsp;·&nbsp;
    <a href="#where-it-stands">Roadmap</a>
  </p>
</div>

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/images/hero-dark.webp">
    <img src="docs/images/hero-light.webp" width="880" alt="Diagram titled 'every task is a branch'. From the point where you ask, three branches grow out of the main line of your real files. World a stops with a cross: two tests fail, thrown away. World c stops with a cross: 40% slower, thrown away. World b, in green, passes every test, reaches a gold diamond labelled 'you say yes', and joins the main line again, which turns green: kept, with the change.">
  </picture>
</p>

> [!NOTE]
> **arbor is not built yet.** This repository holds what comes first: a report of where memory, disk, time, energy and money go on a computer that agents use every day, the tools to measure it again, and a first prototype that runs on today's Linux. The numbers on this page were measured on one PC, except the few marked as estimates and the imagined example of how a task would go.

## In one minute

**An agent** is a program that uses an AI model, the kind behind Claude, ChatGPT or Gemini, to get a job done on its own. You give it a goal, and it does the hundred small steps in between: it reads files, runs programs, searches the web, writes code and, more and more often, spends money.

**An operating system** is the software underneath everything else on a computer: Windows, macOS, Android, Linux. Think of it as the management of a large building. It hands out rooms and keys, keeps the lights and water running, and keeps the list of who may go where.

The trouble is that every operating system in use today descends from ideas of 1969, made for a handful of people typing at terminals. They assume:

- that **one person** sits at the keyboard, when an agent may run hundreds of copies all night;
- that a program may do **anything its user may**, when an agent can be tricked by a single web page it reads;
- that a saved file **replaces** the old one, when an agent wants to try ten ways and quietly throw nine away;
- that programs talk in **text laid out for human eyes**, which an AI has to guess its way through.

**arbor** keeps what made those systems great, a few simple ideas that everything else is built from, and swaps the ideas for ones that fit agents. The good news is that the better ideas are not new: most were built and tested decades ago, and lost only because they asked too much paperwork of people. That paperwork is exactly the patient, exact work that software does without effort.

## What you get

<table>
  <tr>
    <td width="50%" valign="top">
      <h3>💸 A smaller bill</h3>
      Three quarters of what agents cost goes on reading the same words again and again. arbor cuts what they reread before anything else.
    </td>
    <td width="50%" valign="top">
      <h3>⚡ Much faster</h3>
      On the same PC, a safe space for an agent starts about 100 times faster, and a file is found among 2.2 million in a third of a millisecond.
    </td>
  </tr>
  <tr>
    <td width="50%" valign="top">
      <h3>↩️ Undo for everything</h3>
      An agent works on a copy of everything it can see. Keep the result, or throw the copy away whole, with nothing left to clean up.
    </td>
    <td width="50%" valign="top">
      <h3>🔑 Safe to hand over</h3>
      Like a valet key for a car: an agent gets exactly the access its task needs, for as long as it lasts, and never sees your passwords.
    </td>
  </tr>
  <tr>
    <td width="50%" valign="top">
      <h3>📮 Nothing leaves without you</h3>
      Payments, emails and posts wait in an outbox until a rule or a person releases them, and leave exactly once, with a receipt.
    </td>
    <td width="50%" valign="top">
      <h3>🪶 A lighter machine</h3>
      One shared service instead of a heavy program for every agent and a browser for every app, and a third of the disk back from files stored twice.
    </td>
  </tr>
</table>

## The new building blocks

Today's computers are built from users, programs, files and text. arbor is built from these, and it is named after the Latin word for tree because almost everything in it branches.

| the idea | what it replaces | think of it as |
| :--- | :--- | :--- |
| **World** | a disk and its folders | a draft of everything an agent can see, copied in an instant, then kept or thrown away |
| **Object** | a file | a piece of data that knows what it is and is never written over: each change is a new version |
| **Cell** | a program, container or virtual machine | a sealed room where work happens, ready in a fraction of a millisecond |
| **Key** | a user account and its password | a valet key: one job, on one thing, until a set time, within a set budget |
| **Effect** | *(new)* | anything that reaches the outside world, like an email or a payment, held in an outbox first |
| **Channel** | pipes and text | a line between two cells that carries clear, typed messages instead of text to be guessed at |
| **Budget** | *(new)* | what a task may spend: money, time, energy and people's attention |

Under all of them runs **the ledger**, a record of every step every agent takes: what it did, who allowed it, what it read and what it cost.

## One task, start to finish

You ask an agent to update the payment code of a small online shop and put it on the test site, for five dollars at most. It tries three ways at once, each in a world of its own. Checks throw away the two that fail, at no cost to your real files. Nothing has reached the real server or your team's chat yet. What is left is a single card:

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/images/decision-dark.webp">
    <img src="docs/images/decision-light.webp" width="880" alt="A card titled 'one decision, waiting for you'. Payments, moved to version 2: the change is 18 files in the payment code; checked by 214 tests and a type check, all passing; spent $3.41 of $5.00; will leave the machine: a deployment to the test site and a note to the payments team; both can be undone. Buttons: Release both, Refuse, or touch your security key.">
  </picture>
</p>

You say yes, and the change and both messages go out together, or not at all. If a tester finds a problem two days later, one command puts everything back as it was.

## Measured, not guessed

Before building anything, arbor measured where the waste is on one ordinary PC that runs coding agents every day: an Intel Core i7-14700KF with 15 GiB of memory on Ubuntu 26.04, and 201 agent sessions that made 47,700 calls to a model.

**Most of an agent's bill is rereading.** At every step, the model reads the whole conversation again. A cache makes that cheap, and it is still three quarters of the cost:

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/images/bill-dark.webp">
    <img src="docs/images/bill-light.webp" width="880" alt="Stacked bar titled 'three quarters of the bill is reading the same words again'. 74.3%: reading again what the model already read, at every step. 13.8%: the model's own writing. 11.9%: keeping a copy to reread.">
  </picture>
</p>

**The jobs agents do all day can be far faster.** These are the same jobs, on the same PC, done today's way and the arbor way:

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/images/measured-dark.webp">
    <img src="docs/images/measured-light.webp" width="880" alt="Chart titled 'the same jobs, done the arbor way', in milliseconds on a scale where each step is ten times the last. Start a sealed-off space for an agent: 125 ms today, 1.28 ms the arbor way, 98 times faster. Start a Python sandbox: 46.4 ms to 0.73 ms, 64 times. Search all your code for a word: 37 to 174 ms down to 1.4 to 5.3 ms, 26 to 33 times. Find a file by name among 2.2 million: 1,425 ms to 0.32 ms, 4,450 times.">
  </picture>
</p>

<details>
<summary><strong>The same numbers as a table, with a few more</strong></summary>

| the job | today | the arbor way | how |
| :--- | ---: | ---: | :--- |
| Start a sealed-off space for an agent | 125 ms | **1.28 ms** | a Linux namespace instead of a Docker container |
| Start a Python sandbox, ready to work | 46.4 ms, 15.2 MiB | **0.73 ms, 2.1 MiB** | a copy of a warm one instead of a fresh start |
| Search all your code for a word | 37–174 ms | **1.4–5.3 ms** | an index instead of reading every file |
| Find a file by name among 2.2 million | 1,425 ms | **0.32 ms** | a live catalog instead of walking every folder |
| Make a private copy of a 28 GB project to try an idea | | **4.3 ms** | a copy-on-write snapshot: nothing is copied until it changes |
| List 100 files for an agent | 6,283 tokens | **401 tokens** | only the names it asked for, not a page of formatted text |
| Show an agent the last 20 changes to some code | 18,928 tokens | **530 tokens** | only the fields it asked for |
| Disk taken by the projects folder | 70.5 GiB | **47.6 GiB** | each piece of content stored once (a third was exact copies) |
| Time to start the PC | 25.2 s | **≈13 s** (est.) | one kernel image, nothing at boot that can wait, resume instead of restart |

All measured on the PC above, except the boot time after the change, which is an estimate.
</details>

The full report, with every source and the things arbor should refuse to do even where they look faster, is in **[OPTIMIZATION.md](OPTIMIZATION.md)**.

## Try the prototype

The first piece of arbor runs on today's Linux, with no administrator rights. It is one program, `arbor`, and it already has worlds, sealed commands, keys, limits, a network open only to the hosts a key names, the outbox, the ledger and short answers for agents: try a change on an instant copy of a folder, keep it or throw it away whole, and let nothing leave the machine until you say yes.

You need Linux with [bubblewrap](https://github.com/containers/bubblewrap) 0.10 or later (on Ubuntu 26.04, `sudo apt install bubblewrap`) and [Rust](https://rustup.rs). Then, in a copy of this repository:

```sh
cargo install --locked --path prototype/arbor
```

And with any folder you work in:

```sh
arbor new shop --from ~/projects/shop   # an instant copy: nothing is copied until something changes
arbor shell shop                        # work inside it, sealed off; the real folder does not change
arbor diff shop                         # what changed
arbor keep shop                         # put the changes in the real folder; arbor undo takes them back
```

`arbor run shop -- npm test` runs one command instead of a shell. Inside a world your home folder is empty, so tools installed there, such as `~/.nvm`, need a key that opens them: `arbor key new dev --toolchains`, then `arbor run shop --key dev -- npm test`.

A world has no network either, until a key names the hosts it may reach over HTTPS. That lets an agent work in a world of its own: after `arbor key new agent --toolchains --read ~/.local/share/claude --env ANTHROPIC_API_KEY --net api.anthropic.com`, Claude Code started in `arbor shell shop --key agent` reaches its API and nothing else, and what it changes waits in the world until you keep it.

An agent asks for what it reads with `arbor look`: several reads in one call, each a short table of only the fields it wants. `arbor look status + log -n 5 + files src` answers what changed, the last five commits and what a folder holds in one step instead of three, and `arbor look` alone describes all it answers, so the agent's prompt needs no tool description for it.

How fast it is on the PC above. Every number here is **M**: the suite's [`arbor` section](bench/README.md#sections) measured it on 17 October 2026, except the output of `ls -la`, `git log -20` and `ps aux` in brackets, which the report measured the day before, and the last column names its row in [`bench/lib/reference.py`](bench/lib/reference.py).

| the job | the prototype | suite row |
| :--- | ---: | :--- |
| Make a world: an instant copy of a folder | 2.0 ms | `arbor.new_ms` |
| Start a command in it, sealed off | 5.1 ms | `arbor.run_cgroup_ms` |
| Start it with the network open to one host | 5.7 ms | `arbor.run_net_ms` |
| From inside, connect through arbor to a server on this PC (directly: 0.03 ms) | 0.15 ms | `arbor.tunnel_ms`, `arbor.tunnel_direct_ms` |
| From inside, ask for something only you can allow, such as a `git push` | 0.6 ms | `arbor.ask_ms` |
| Run it once you say yes, with a receipt | 1.5 ms | `arbor.release_ms` |
| Put 1,000 new files in the real folder, or take them back out | 6.2 ms, 4.1 ms | `arbor.keep_ms`, `arbor.undo_ms` |
| Tell an agent what changed in 100 files | 700 tokens | `arbor.diff100_tokens` |
| Show an agent what a folder of 100 files holds (`ls -la`: 2,776 tokens) | 422 tokens | `arbor.look_files_tokens` (`fmt.ls_human`) |
| Show an agent the last 20 commits (`git log -20`: 18,928 tokens) | 579 tokens | `arbor.look_log_tokens` (`fmt.git_log`) |
| Show an agent the 30 programs using the most memory (`ps aux`: 9,900 tokens) | 325 tokens | `arbor.look_procs_tokens` (`fmt.ps_aux`) |
| Show an agent 15 changes in a repository (`git status`: 244 tokens) | 128 tokens | `arbor.look_status_tokens`, `arbor.git_status_tokens` |
| Answer what changed, the last commits and a folder's files in one call | 2.7 ms | `arbor.look_ms` |
| Describe to an agent all that `arbor look` answers | 309 tokens | `arbor.look_about_tokens` |
| Memory of the one background service it keeps running | 2.7 MiB | `arbor.serve_rss_mib` |

A sealed command starts about 24 times faster than the Docker container above (125 ms), though not yet in the 1.28 ms of a plain Linux namespace: the prototype also mounts the world over the folder, sets up the command's limits, filters the kernel calls it may make, and records each step in the ledger. Opening the network to one host adds 0.6 ms to the start, and passing through arbor 0.12 ms to each connection. `arbor look` answers in 3–15% of the tokens of `ls -la`, `git log -20` and `ps aux`, and in about half those of `git status`, which is already short. Asking for three reads at once saves more: two of the steps that make an agent reread its whole conversation, which cost about 42,000 input-token equivalents each on the PC above (**M**). What a command in a world can and cannot do, and how keys, the network, limits, the outbox, the ledger and `arbor look` work, is in **[prototype/README.md](prototype/README.md)**.

## Measure your own PC

The measurements are a suite of scripts you can run on a Linux PC. It needs no administrator rights, only reads your files, writes nothing but a temporary folder and its own report, and takes about two and a half minutes. With Rust set up for the prototype, it also builds the prototype in that temporary folder and measures it the same way, in about 15 seconds more; if the prototype's background service is not running, it starts it for those rows and stops it again. Last, on a Wayland desktop, it spends about two minutes on the screen: for one minute nothing should touch the PC, then a small window of its own animates, first on a quiet PC and then with every core busy and most of the free memory taken:

```sh
git clone https://github.com/NahuelGenchi/arbor.git
cd arbor/bench
./run.sh
```

It writes a report to `bench/results/` with every number next to the one measured for arbor, and why any of them differ. Run only some parts with `ONLY="boot search" ./run.sh`. What it needs, what it reads and how to read the report are in **[bench/README.md](bench/README.md)**.

> [!TIP]
> Close other programs first. A busy PC reads 20–40% slower, and the report says when that happened.

## Where it stands

arbor is planned in steps. The first prototype is a layer on top of Linux, so today's programs keep working inside it; the whole of it, with new hardware, is a project of about ten years. Each step is a milestone:

- 🟢 **[M0 · Project foundation](https://github.com/NahuelGenchi/arbor/milestone/1)** — this repository, its documents and how work is tracked
- 🟢 **[M1 · Measurements](https://github.com/NahuelGenchi/arbor/milestone/2)** — the report and the suite; the baseline of 16 October 2026 is done, and the suite measures the prototype too
- 🟢 **[M2 · Worlds and storage](https://github.com/NahuelGenchi/arbor/milestone/3)** — instant copies of everything, each piece of content stored once, finding anything at once. *In the prototype: worlds of a folder, kept or undone whole*
- 🟢 **[M3 · Cells](https://github.com/NahuelGenchi/arbor/milestone/4)** — sealed rooms that start in a fraction of a millisecond, and the keys that open them. *In the prototype: sealed commands with limits, keys, a filter on kernel calls, and a network open only to the hosts a key names*
- 🟢 **[M4 · Agent layer](https://github.com/NahuelGenchi/arbor/milestone/5)** — agents that reread less: tools described on demand, short typed answers, one shared service. *In the prototype: `arbor look`, several reads in one call, each a short table*
- ⚪ **[M5 · Lean system](https://github.com/NahuelGenchi/arbor/milestone/6)** — memory, starting up and energy: no swapping to disk, no waiting at boot, nothing that polls
- 🟢 **[M6 · Outbox and ledger](https://github.com/NahuelGenchi/arbor/milestone/7)** — effects that wait for a yes, budgets, and the record of every step. *In the prototype: the outbox, budgets in money, and the ledger*

<sub>🟢 in progress &nbsp;·&nbsp; ⚪ planned</sub>

## Read more

<table>
  <tr>
    <td width="50%" valign="top">
      <a href="https://nahuelgenchi.com/software/arbor/"><strong>The operating system agents need</strong></a><br>
      <sub>Part 1. The proposal: why today's systems cannot simply be patched, the seven building blocks, keys and the outbox, and how it could be built one step at a time.</sub>
    </td>
    <td width="50%" valign="top">
      <a href="https://nahuelgenchi.com/software/arbor-measured/"><strong>arbor, measured</strong></a><br>
      <sub>Part 2. Where one computer's memory, time and tokens really go, and what arbor should build first, take out and refuse because of it.</sub>
    </td>
  </tr>
</table>

## In this repository

| path | what it is |
| :--- | :--- |
| [`OPTIMIZATION.md`](OPTIMIZATION.md) | The report: the waste measured today, the top 10 changes for the prototype, and every source |
| [`bench/`](bench/) | The suite that measures it all again, section by section, and the prototype the same way |
| [`prototype/`](prototype/) | The first prototype, on today's Linux: one program, `arbor`, with worlds, sealed commands, keys, limits, a network open only to the hosts a key names, the outbox, the ledger and short answers for agents |
| [`docs/INTERFACE.md`](docs/INTERFACE.md) | The design brief for Mission Control, the screen where people will direct agents. No interface exists yet: this is the plan for one |
| [`docs/images/`](docs/images/) | The pictures on this page, and the pages they are drawn from |

Work is tracked from issue to branch to milestone to pull request; see [CONTRIBUTING.md](CONTRIBUTING.md) and [docs/TRACKING.md](docs/TRACKING.md).
