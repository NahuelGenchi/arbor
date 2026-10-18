# Mission Control: design brief

[Back to README](../README.md) · [Issue #6](https://github.com/NahuelGenchi/arbor/issues/6) · [M7 — Interface](https://github.com/NahuelGenchi/arbor/milestone/8)

> [!NOTE]
> **Only the first app exists.** This brief decides what Mission Control shows and how a person directs agents through it. The first app ([#9](https://github.com/NahuelGenchi/arbor/issues/9), [`prototype/mission/`](../prototype/mission/README.md)) is one window in GTK 4. It shows the tries, the rooms working in them, the decisions waiting and the last steps of history, and it answers decisions with Release and Refuse ([#82](https://github.com/NahuelGenchi/arbor/issues/82)) and keeps a try or undoes the keep ([#83](https://github.com/NahuelGenchi/arbor/issues/83)). The other views below are still plans, and the pictures are wireframes with made-up data. [#8](https://github.com/NahuelGenchi/arbor/issues/8) measured a small test program in five toolkits, and GTK 4 was chosen ([The toolkits measured](#the-toolkits-measured)). #9 measured the first app against the targets it can meet without input ([The first app, measured](#the-first-app-measured)).

Mission Control is the screen where people direct agents. It shows what agents are changing, what they want to send out and what they have spent, and it asks for a person only when a person is needed. Today's desktops were built for one person and a handful of windows. Mission Control has to make hundreds of agents legible to one person at a glance, and spend that person's attention as carefully as money.

It is for people only. Agents never read the screen: they read arbor's typed state directly ([OPTIMIZATION.md §6](../OPTIMIZATION.md#6-agent-tokens-cut-steps-and-context-first)). Nothing on screen is there for a machine, so all of it can be written for someone who has never written code. Every view is judged by one test: can that person tell at a glance whether anything needs them?

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="images/mission-now-dark.webp">
    <img src="images/mission-now-light.webp" width="880" alt="Wireframe titled '312 agents, nine lines, three that need you': the Now view of Mission Control. A heading says 3 things need you; 312 agents are working on 9 tasks, and everything else is going to plan. Under 'needs you', oldest first, each with a Decide button: a decision on Payments, moved to version 2, going to the test site and the payments team, which can be undone; a decision on the March newsletter, an email to 1,204 subscribers, which can't be undone; and Photo archive, sorted by year, stopped at its $8 limit with 2,140 of 3,000 photos sorted. Under 'working, 6 tasks, 297 agents', one line per task: Help pages in six languages, 180 of 240 pages translated, 96 agents until 17:00; Test suite made faster, trying 8 ways with 3 thrown away, 64 agents; Bug reports sorted, 412 of 560, 58 agents; Website speed-up, trying 6 ways with 2 thrown away, 48 agents; Supplier invoices filed, 286 of 900, 30 agents; and Weekly sales summary, 1 agent, idle while it waits for the bookkeeping export. Then: done today, 5 tasks. A side panel shows today's budgets: money $41 of $60, memory for agents 5.2 of 9 GB, attention 3 of 10 asks, and the last task ends at 18:00. When money runs out, new model calls wait for you.">
  </picture>
</p>

## Six rules every view follows

1. **Only what needs you rises.** Decisions, stops and requests come first. Everything else waits in its view until someone looks.
2. **A task is a line, not an agent.** 312 agents working on nine tasks are nine lines. Agents appear only when you open a task.
3. **Facts first, the agent's words second.** What will happen, to whom, at what cost and whether it can be undone comes from arbor's own records. What the agent says about it comes after, marked as its own words: an agent can be wrong, or tricked by a page it read.
4. **The screen changes when its meaning changes.** No streams, no spinners, no ticking clocks. A still screen does no work ([OPTIMIZATION.md §5](../OPTIMIZATION.md#5-energy-at-the-same-quality)).
5. **No private path.** Every button sends the same typed request an agent would, through the same keys and the outbox's own rules, and leaves a step in the ledger with you as the actor.
6. **Plain words.** Clock times, units and whole sentences on the first layer; arbor's own terms one click down.

## Words on screen

Mission Control uses everyday words, with arbor's own terms one click down:

- a **try** (a world): a copy of your files that an agent works on, kept or thrown away whole
- a **room** (a cell): where an agent's work runs, sealed off from everything else
- a **key**: what an agent may open, until when, and how much it may spend
- a **decision** (an outbox item, while it waits): something that will leave the machine, such as a deployment, an email or a payment
- **history** (the ledger): every step every agent took, in order
- a **budget**: money, time, memory and your attention
- a **task** (no object of its own yet): what you asked for, which can hold many tries, rooms and agents

## The views

Each view names the typed objects it reads: worlds (M2), cells and keys (M3), and the outbox, ledger and budgets (M6). The core is planned to publish them over a Unix socket (`arbor watch`) that Mission Control subscribes to, with no polling. The fields below are the ones planned for the core in M2–M6; where a view needs more, it is listed under [open questions](#open-questions). Worlds already exist in the prototype ([#14](https://github.com/NahuelGenchi/arbor/issues/14)). A world there is open or kept, and throwing it away deletes it, so the tries that were thrown away, and why, come from the ledger.

### 1. Now

The first screen, and the one most people will use most. Top to bottom:

- **One sentence** that answers the question the screen exists for: *3 things need you*, or *Nothing needs you*, with how many agents are working on how many tasks.
- **Needs you**: decisions waiting, tasks stopped by a budget, and agents asking for a wider key, oldest first. Each is one line that opens its decision.
- **Working**: one line per task, with how far it has got in words that change rarely (*180 of 240 pages translated*, *trying 6 ways, 2 thrown away*), how many agents, and until when.
- **Done today**: one line with a count, which opens History.
- **Budgets**: today's money, memory and attention, spent against the limit, and when the last task ends.

**Reads:** outbox items (waiting, with their cost); worlds (open or kept) and the ledger (tries thrown away), to count tries; cells (running or idle, to count agents, and memory used against limits); keys (to group rooms into tasks, and their expiry); budgets (money, time, memory, attention).

### 2. Decision

One outbox item and everything needed to answer it, in one place:

- **What will leave the machine**, where it goes, what it costs, and whether it can be undone: facts from the outbox item.
- **The change it carries**, if any: the try and how many files it changes, which opens Changes.
- **What checked it**: the checks and their results, from the ledger.
- **What it has spent** of its budget.
- **In the agent's own words**: its summary, shown last and marked as not checked.
- **The steps behind it**: the ledger entries that produced it, oldest first, with a way to the whole history.

**Release** and **Refuse** answer it in one gesture with all of this in view. An effect that cannot be undone, or that costs more than a limit the person sets, also asks for a second confirmation, such as a touch of a security key. A release goes through the outbox's own rules and leaves its receipt in the ledger; Mission Control has no other way to send anything off the machine ([#12](https://github.com/NahuelGenchi/arbor/issues/12)).

*In the first app* ([#82](https://github.com/NahuelGenchi/arbor/issues/82)), the decision is a line of Now rather than a view of its own: its facts, the asker's words, and Release and Refuse. A press opens one confirmation in place, with the focus on Cancel, and the window then runs `arbor release` or `arbor refuse`, the command a person would type. The receipt replaces the decision in a list of answers. The second confirmation waits for the outbox to record whether an effect can be undone ([open question 3](#open-questions)).

**Reads:** an outbox item (the effect, its target, its cost, waiting, released or refused, and the ledger steps behind it); those ledger entries; the world they worked in; budgets.

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="images/mission-decision-dark.webp">
    <img src="images/mission-decision-light.webp" width="880" alt="Wireframe titled 'everything you need to say yes, in one place': the decision on Payments, moved to version 2, waiting for you, the first of 3 things that need you. Facts first: will leave the machine, a deployment to the test site and a note to the payments team; can be undone, both; the change, 18 files in the payment code, try b of 3, with a link to see the changes; checked by 214 tests and a type check, all pass; spent $3.41 of $5.00. Then, marked as the agent's own words and not checked: moved the payment code to version 2, and refunds keep the old rules until the team says otherwise. Buttons: Release both, Refuse, or touch your security key. Beside them, the steps behind it: at 13:52 you asked for it with $5 to spend; at 13:52 three tries began, each on a copy of the shop's code; at 13:58 try a failed 2 tests and was thrown away; at 14:01 try c was 40% slower and was thrown away; at 14:05 all 214 tests passed on try b; at 14:06 try b asked to deploy and send a note. A link leads to all 1,212 steps in History.">
  </picture>
</p>

### 3. Changes

What a task is changing, try by try ([#11](https://github.com/NahuelGenchi/arbor/issues/11)):

- **The tries side by side**: passing, still working, or thrown away with the reason (*12% slower*, *tests failed*).
- **For the chosen try**, the files changed against the folder it was copied from, grouped by folder and shown outline-first: for code, the functions and sections that changed come before any line, and the lines are one click down.
- **Keep** and **Throw away** act on the whole try, never on part of it. Keep is refused if your own files changed since the try began: the view lists those files and offers a new try from your current files instead. A kept try can be undone.

**Reads:** a world (its name, the folder it was copied from, the files changed, and whether it is open or kept), and the ledger for the tries thrown away and why. The outline needs the outline-first code objects of M2; until they exist, the view shows the files added, changed and deleted, and their changed lines, as `arbor diff` and `arbor diff --patch` give them ([#14](https://github.com/NahuelGenchi/arbor/issues/14)).

*In the first app* ([#83](https://github.com/NahuelGenchi/arbor/issues/83)), a try's changes open under its line in Now: its files in words, by folder, each text file with up to 40 of its changed lines one click down. Keep asks one confirmation in place, as Release does, then runs `arbor keep`; a kept try has Undo the keep, which runs `arbor undo`. When your own files changed since the try began, the view says first that a keep would be refused, and a keep that arbor refuses names those files in plain words. A new try from your current files, and Throw away, are not there yet.

### 4. Room and keys

Where a try's work runs, and what it may touch:

- **Each room**: working or idle, the try it works in, and the processor and memory it uses against its limits.
- **Each key it holds**: what it opens, until when (a clock time, never a countdown), and how much it may spend, with how much is spent.
- **Stop** ends a room; **Take back** ends a key. An agent that wants a wider key asks in Needs you; nothing is ever widened silently.

**Reads:** cells (the world, the key, running or idle, CPU and memory used against limits) and keys (what each may read, its expiry, its budget).

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="images/mission-task-dark.webp">
    <img src="images/mission-task-light.webp" width="880" alt="Wireframe titled 'what a task changes, and what it may touch': the task Website speed-up, working, with 48 agents, $2 of $6 spent, until 18:00. A row of six tries, each on its own copy of the site's code: try a, 31% faster, is selected; tries b, c and f are working; try d, 12% slower, and try e, with failed tests, were thrown away. Try a's 9 changed files, kept or thrown away whole, grouped by folder and shown outline-first: in pages, product.html with its gallery and reviews sections changed, and 3 more pages with the same change; in scripts, gallery.js with a new loadWhenVisible and a changed showGallery, and 2 more scripts; in styles, main.css with its gallery and header rules changed, and print.css. Buttons: Keep try a, and Throw it away; keeping applies all 9 files at once and can be undone. A side panel shows try a's room, idle since 14:20, its processor idle with a limit of 1 core and its memory at 180 of 512 MB, and its key, which opens the site's code to read, until 18:00 today, and may spend $1.00, of which $0.62 is spent. What it changes stays in try a until you keep it, and anything that would leave the machine waits for you first. Buttons: Stop the room, and Take back the key.">
  </picture>
</p>

### 5. History

The ledger as a timeline:

- **One sentence per step**: when, who, with which key, did what, to what, with what result, at what cost.
- **A day to scrub through**, by dragging or with the arrow keys, drawn with where the steps cluster; filters by task, try or agent, or only the steps that reached outside the machine.
- **The list holds still.** It shows the moment you opened it. New steps never push it along: one still line says that there are newer steps, and shows them when you ask.

**Reads:** ledger entries (sequence, time, actor, key, action, target, result, cost), a page at a time from the core rather than all held in memory.

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="images/mission-history-dark.webp">
    <img src="images/mission-history-light.webp" width="880" alt="Wireframe titled 'every step, and the list holds still': the History view, with filters for all tasks, all agents and only what left the machine, and 48,120 steps today. A still bar chart of the day's steps from 08:00 to 14:00 has a line at 14:05 to drag, or to move with the arrow keys. A dashed notice says there are newer steps than these, with a Show them link. Five steps follow, newest first, each with its time, task, agent, action, step number, key, result and cost: at 14:05:58 in Payments, try b's agent asked to deploy to the test site, waiting for you, $0.00; at 14:05:51 in Help pages, agent 41 translated 'Returns and refunds' into Spanish, done, $0.02; at 14:05:40 in Payments, try b's agent ran 214 tests, all pass, $0.04; at 14:05:31 in Website speed-up, try d's agent measured how fast the pages load, 12% slower and thrown away, $0.01; at 14:05:12 in Bug reports, agent 7 read report 412 and sorted it under checkout, done, $0.01.">
  </picture>
</p>

### 6. Budgets

What is left to spend:

- **Money, time, memory and attention**, each spent against its limit, for today and for each task.
- **What happens when one runs out**, in plain words. Proposed: new model calls wait for a person; a task that reaches its time ends and keeps its tries for you to look at; the newest tries pause when memory runs short; decisions keep waiting in Now but stop interrupting you. M6 decides the rules, and this view says them.
- **Attention is counted in asks.** Each decision or stop that interrupts a person spends one, so agents have a reason to merge their questions into one decision, as in the README's example.

**Reads:** budgets (money, time, memory, attention), and keys for each task's share.

## Hundreds of agents, a few lines

Nobody reads 300 streams. Mission Control shows no live stream per agent; it turns them into a few lines:

- **Group by task.** A task is what a person asked for. Everything done under the key the person handed out, or under keys narrowed from it, belongs to one line: its tries, its rooms and its agents.
- **Rank by need.** Needs you (decisions, stops and requests), then working, then idle, then done. Within a band, what has waited longest comes first.
- **Summaries, not logs.** Each line is a short sentence of counts and states: *trying 8 ways, 3 thrown away*, not the last thing an agent printed.
- **Numbers at the precision of a decision.** Whole dollars on Now, cents on the decision itself; counts of finished things rather than rates or percentages that change every moment.
- **Fold what is normal.** When tasks don't fit, the quiet ones fold into one line: *and 14 more tasks, 210 agents, all working normally*.
- **Lines stay put.** A line moves only when its band changes, never because a number changed. When a line rises, whatever is under the pointer and the keyboard focus stays where it is, so a click never lands on something that moved there a moment before.
- **Interruptions are spent like money.** Only what needs a person raises a notification. Several at once become one (*3 decisions are waiting*), quiet hours hold them, and a notification opens its decision but never releases anything by itself.

### How an update reaches the screen

1. **Subscribe, don't poll.** Mission Control subscribes to the core's typed state and sleeps until the socket has news or the person acts.
2. **Subscribe to what is shown.** Now asks for summaries per task: counts, states and totals. Opening a task adds its tries, rooms and keys, and closing it drops them. History reads the ledger a page at a time.
3. **Fold, then draw once.** Changes are folded into the app's state as they arrive, and drawing waits for the next frame, so a hundred changes between two frames make one frame. Only the lines whose words changed are laid out and repainted.
4. **No change, no frame.** A hidden view or window draws nothing, and keeps only the subscriptions that can raise something that needs a person.

| what changed | when it is drawn |
| :--- | :--- |
| your own input: a key, a click, a scroll | the next frame, always |
| something that needs you | the next frame |
| a view you opened: a task, a room, history | the next frame, folded with everything else that arrived |
| a summary on Now: counts, money, memory | at most once a second, and only if its words change |

The one-second wait on Now is armed by a change, never by a clock, so a quiet system has no timer running at all.

## Motion and effects

- **Nothing moves on a still screen.** No spinners, pulsing dots, shimmering placeholders, animated progress bars, ticking clocks or countdowns: work in progress is a word and a still mark. Times are clock times (*since 14:02*, *until 18:00*), never *3 min ago* or *2 h left*, which would redraw the screen every minute to stay true. A text caret stops blinking soon after typing stops.
- **Motion answers an event, once.** A decision arriving, a line moving into Needs you, a try kept or thrown away, a view opening. Nothing animates on a timer or to catch the eye.
- **Short and interruptible.** 100 to 200 ms, leaving faster than arriving, with no bounce. Any input or newer event takes over from wherever the motion is. Input never waits for an animation, and the result can be used before the motion ends.
- **Many changes, one motion or none.** Lines that change in the same frame move together. Past a handful, they change in place without moving: nobody can follow twenty moving things.
- **Only what a compositor can play.** Motion changes only the opacity and position of things already drawn, so it needs no new layout, and the compositor can later play it on its own ([#13](https://github.com/NahuelGenchi/arbor/issues/13)).
- **Blur and translucency only over still content.** A sheet over Now blurs it once and keeps the result. If what lies underneath is changing, the backdrop is plain instead of a blur redrawn every frame. Both are off when the desktop asks for less transparency or more contrast.
- **Reduced motion** turns every movement into an instant change or a fade of at most 100 ms. Nothing needs motion to be understood.

## Accessibility

- **Keyboard.** Everything the pointer can do, the keyboard can do too. Tab moves between areas and the arrow keys within a list; Enter opens; Escape goes back and returns the focus to the line you came from; Ctrl+K finds any task, try, agent or file; and ? lists every shortcut. The focus is always visible. No single key sends anything off the machine: Release is a button like any other, and an effect that cannot be undone also asks for a second confirmation.
- **Screen readers.** Every control has a role, a name and a state on AT-SPI, the Linux accessibility bus ([#8](https://github.com/NahuelGenchi/arbor/issues/8) records each toolkit's support). Each line reads as one sentence: *Payments, moved to version 2. Decision waiting: to the test site and the payments team. $3.41 of $5 spent. Can be undone.* Only what needs a person is announced, politely and merged (*Two decisions are waiting*), never routine progress.
- **Reduced motion.** The desktop's setting is followed, as described under motion.
- **Contrast.** Text has at least 4.5:1 contrast against its background ([WCAG 2.2](https://www.w3.org/TR/WCAG22/#contrast-minimum), level AA). Focus rings, status marks and the fill of a meter have at least 3:1 against what is behind them ([non-text contrast](https://www.w3.org/TR/WCAG22/#non-text-contrast)). A state is never shown by colour alone: it always has a word, and a mark with a shape of its own. In the wireframes, the faintest text computes to at least 4.7:1 in both themes, the main button to 6.8:1 in light and 8.9:1 in dark, and every status mark and meter fill to at least 3:1.
- **Light and dark.** Mission Control follows the desktop's light or dark setting, and a person can override it. Both themes share one layout and differ only in colour, and every view is drawn and checked in both, as the wireframes are. A high-contrast setting replaces washes and faint text with full ink.
- **Size.** The desktop's text size is followed. At twice the size, or in a narrow window, the layout drops to one column (needs you, then working, then budgets) instead of cutting text. Click targets are at least 24 × 24 px, and the main actions larger.

## Targets

These are targets for the issues that build and measure Mission Control. #8's test program was measured against them ([The toolkits measured](#the-toolkits-measured)), and so was the first, read-only app, except where a target needs input or views it does not have yet ([The first app, measured](#the-first-app-measured)). Each issue records its numbers, marked **M**, with the machine and its conditions.

- **Missed frames.** *Target:* none with the machine idle, and at most 1 in 1,000 under load (300 agents in cells plus a memory spike), at 60 Hz and at 120 Hz. Checked by the frame-pacing rows of [#7](https://github.com/NahuelGenchi/arbor/issues/7), then by [#9](https://github.com/NahuelGenchi/arbor/issues/9) to [#12](https://github.com/NahuelGenchi/arbor/issues/12) with 300 agents.
- **Work per frame.** *Target:* the app's own work for a frame takes at most 4 ms for 99% of frames: a quarter of a frame at 60 Hz (16.7 ms) and half of one at 120 Hz (8.3 ms), so the compositor and the GPU keep the rest. Checked by the test program of [#8](https://github.com/NahuelGenchi/arbor/issues/8), then by [#9](https://github.com/NahuelGenchi/arbor/issues/9).
- **Input latency.** *Target:* an input is answered in the next frame. From the input event to the frame that shows its result takes at most 2 refreshes for 99% of inputs (33 ms at 60 Hz, 17 ms at 120 Hz), idle and under load. Checked with the presentation timing of [#7](https://github.com/NahuelGenchi/arbor/issues/7), by [#9](https://github.com/NahuelGenchi/arbor/issues/9) and [#10](https://github.com/NahuelGenchi/arbor/issues/10). Input to photon, from the device to the panel, needs a camera or a hardware tester and is out of scope in #7.
- **Still screen.** *Target:* with nothing changing, 0 frames drawn, at most one wakeup every 10 s, and at most 0.1% of a core over 60 s. Today, gnome-shell alone uses 2.1% of a core on a still screen (M, [OPTIMIZATION.md §1](../OPTIMIZATION.md#1-ram-cpu-gpu)). Checked by the still-screen row of [#7](https://github.com/NahuelGenchi/arbor/issues/7), then by [#9](https://github.com/NahuelGenchi/arbor/issues/9).
- **Busy but calm.** *Target:* with 300 agents working and nothing needing a person, at most 1 frame a second and 1% of a core. Checked with #7's method by [#9](https://github.com/NahuelGenchi/arbor/issues/9), [#11](https://github.com/NahuelGenchi/arbor/issues/11) and [#12](https://github.com/NahuelGenchi/arbor/issues/12).
- **Memory.** *Target:* at most 128 MiB RSS and 64 MiB PSS with 300 agents and every view opened once, under a tenth of the Electron UI measured today (1,354 MiB RSS, M, [OPTIMIZATION.md §1](../OPTIMIZATION.md#1-ram-cpu-gpu)); within 10% of the same app with 3 agents; and no growth over an hour of updates. Checked by [#8](https://github.com/NahuelGenchi/arbor/issues/8) and [#9](https://github.com/NahuelGenchi/arbor/issues/9).
- **First frame.** *Target:* at most 200 ms from launch to the first frame that shows real state, with warm caches. Checked by [#8](https://github.com/NahuelGenchi/arbor/issues/8) and [#9](https://github.com/NahuelGenchi/arbor/issues/9).

## The toolkits measured

[#8](https://github.com/NahuelGenchi/arbor/issues/8) measured the same small test program in five toolkits that draw on the GPU and run on today's Wayland desktops: GTK 4, Qt Quick, Slint, iced and Flutter. The program is one window with a heading, a card and a list of 300 rows that change, an animation of the card, then a still screen, all with made-up data ([`bench/toolkits/`](../bench/toolkits/README.md)). It is not Mission Control: the numbers show what each toolkit costs before any app code. `ONLY=toolkits bench/run.sh` measures them again ([`bench/README.md`](../bench/README.md)).

**Machine and conditions.** Intel Core i7-14700KF (8 P-cores and 12 E-cores, 28 threads), 15 GiB of RAM, an AMD Radeon R9 380 with Mesa 26.0.8, Ubuntu 26.04.1 with kernel 7.0 and GNOME on Wayland at 60 Hz. There were five `ONLY=toolkits` runs on 17 October 2026, between 19:54 and 22:28, each from an ordinary process (a systemd user unit). They started at load averages of 0.02 to 0.57. Each figure is the median of 3 or 4 clean samples, with their range in brackets. A sample is clean when:

- no key or mouse was used;
- the window kept the focus and was shown for at least 90% of the refreshes;
- the other programs used less than one CPU thread in all;
- on the still screen, the compositor woke fewer than 400 times a second;
- the load average was under 1 at its start.

Samples taken while a game was open in another window, someone was at the PC or another session was compiling were left out, and the suite now fails such rows itself. In the clean samples, the busiest other program, a desktop app, used about 10% of one CPU thread. The renderers are the ones each toolkit picked on this PC: GTK's OpenGL renderer (it turns Vulkan down here), Qt's scene graph on OpenGL, FemtoVG on OpenGL for Slint, wgpu on Vulkan for iced, and Impeller on OpenGL ES for Flutter.

| Measure (M: every number in the row) | GTK 4 | Qt Quick | Slint | iced | Flutter | Target |
| :--- | ---: | ---: | ---: | ---: | ---: | :--- |
| First frame, ms (M) | 170 (167–180) | 147 (131–148) | 93 (92–98) | 99 (96–106) | 199 (197–214) | at most 200 |
| Memory 2.5 s after the start, RSS / PSS, MiB (M) | 140 / 118 | 162 / 138 | 94 / 80 | 95 / 84 | 174 / 139 | |
| Memory after a minute of still screen, RSS / PSS, MiB (M) | 163 / 140 | 165 / 141 | 94 / 80 | 95 / 84 | 155 / 132 | at most 128 / 64 for the whole app |
| Still screen: CPU, % of one core (M) | 0.00 (0.00–0.13) | 0.00 (0.00–0.02) | 0.02 (0.00–0.02) | 0.02 (0.00–0.02) | 0.00 (0.00–0.02) | at most 0.1 |
| Still screen: wakeups a second (M) | 0.10 (0.10–0.55) | 0.05 (0.05–0.05) | 2.0 (2.0–2.0) | 4.0 (4.0–4.0) | 0.30 (0.30–0.32) | at most 0.1 |
| Animation, machine idle: frames missed, % (M) | 0.25 (0.17–0.33) | 0.25 (0.17–0.42) | 0.33 (0.25–0.33) | 0.25 (0.17–0.25) | 0.25 (0.25–0.42) | none |
| Animation, machine idle: slowest 1% of frame times, ms (M) | 16.7 | 16.7 | 16.7 | 16.7 | 16.7 | |
| Animation: work per frame, slowest 1%, ms (M) | 0.97 (0.88–0.99) | 0.66 (0.56–0.69) | 0.92 (0.91–1.02) | 1.64 (1.44–1.85) | 2.59 (2.35–2.80) | at most 4 |
| Animation under load: frames missed, % (M) | 2.2 (1.3–10.8) | 11.4 (10.1–16.1) | 83.3 (83.3–83.3) | 16.3 (14.1–16.7) | 4.3 (3.2–6.1) | at most 0.1 |
| Animation under load: slowest 1% of frame times, ms (M) | 33.3 | 33.3 | 133 (117–150) | 33.3 | 33.3 | |
| Objects on the accessibility bus (M) | 127 | 109 | 69 | 0 | 42 | |
| Heading, card and the rows' words readable on AT-SPI | yes | yes, with Qt Quick Controls' `Label` | yes, through AccessKit | no: not on the bus | yes, through its GTK 3 runner | every control |
| The animation runs on | the main thread, from the window's frame clock | the render thread (Qt Quick's Animators) | the main thread | the main thread, which builds the whole view again each frame | the UI thread (Dart) ticks it, the raster thread draws | |
| Licence | LGPL-2.1-or-later | LGPL-3.0 or GPL-2.0, or commercial | GPL-3.0-only, or Slint's royalty-free or commercial licence | MIT | BSD-3-Clause | |
| Versions | GTK 4.22.4, C | Qt 6.10.2, C++ and QML | Slint 1.18.1, Rust | iced 0.14.0, Rust | Flutter 3.47.7, Dart 3.13.5 | |

"Under load" is the screen section's load: a busy cell on every CPU thread (28 here) and a memory spike. Measured the same way on this PC, that section's own minimal Wayland client missed 0.13% of its frames idle and 8.35% under load (M, [`bench/lib/reference.py`](../bench/lib/reference.py)). The targets are for the whole app with 300 agents, and the test program is far smaller, so they bound it only from below. A toolkit that misses a target here would miss it in the app as well.

What the table shows:

- **No toolkit hands an animation to the compositor.** Each one commits a new buffer for every frame: about 1,197 commits in the 1,199 refreshes of each idle run.
- **Machine idle, every toolkit keeps up.** Each misses 2 to 5 of about 1,200 refreshes, close to the minimal client's 0.13%, so none meets "none".
- **Under load, none meets 1 in 1,000.** GTK 4 (2.2%) and Flutter (4.3%) missed fewer frames than the minimal client did, and Qt Quick (11.4%) and iced (16.3%) missed more. Slint fell to 10 frames a second, the rate of the program's 100 ms timer for the rows. Counting its commits without the probe (`WAYLAND_DEBUG`) gave the same 10 a second. Keeping time and memory for the screen is [#10](https://github.com/NahuelGenchi/arbor/issues/10)'s work, and it may change these numbers for every toolkit.
- **On a still screen, only GTK 4 and Qt Quick meet the wakeup target.** Flutter wakes 0.3 times a second. Slint and iced wake 2 and 4 times a second, from threads named `blocking` that each wake every 500 ms. The `blocking` crate starts them for zbus, the D-Bus library that Slint uses for AccessKit and iced for reading the desktop's settings. All five stay under 0.1% of a core.
- **Memory.** Only Slint (94 MiB RSS) and iced (95 MiB) fit within the whole app's RSS target. GTK 4, Qt Quick and Flutter use 155 to 165 MiB for the test program alone. None is within 64 MiB of PSS. In the run of 21:33, files mapped into each program (libraries, Mesa's driver among them) made up 52 to 97 MiB of its PSS (M).
- **Work per frame** is within 4 ms for all five, from 0.66 ms (Qt Quick) to 2.59 ms (Flutter). **The first frame** takes under 200 ms for all five, though Flutter is at the limit.
- **Accessibility.** iced 0.14 puts nothing on the accessibility bus, which rules it out under [Accessibility](#accessibility). With Qt 6.10, a plain `Text` in Qt Quick does not reach the bus; `Label` does ([`bench/toolkits/README.md`](../bench/toolkits/README.md#accessibility)).

**Decided: GTK 4**, through the gtk4-rs bindings, since arbor's prototype is written in Rust. The owner chose it on 17 October 2026, from this table. GTK 4 is the only toolkit here that meets the still-screen target and also missed the fewest frames under load. Its support for AT-SPI is complete, its licence is the LGPL, and it is the desktop's own toolkit on GNOME.

Two points stayed open: the test program alone used 163 MiB of RSS and 140 MiB of PSS, above the target of 128 MiB and 64 MiB before any app code, and gtk4-rs was not measured, since the test program is written in C. [#9](https://github.com/NahuelGenchi/arbor/issues/9) answers both ([The two points #8 left open](#the-two-points-8-left-open)).

The alternatives, not chosen:

- **Slint** is the lightest in memory and the fastest to start. Its 10 frames a second under load and its wakeups on a still screen would need fixing, upstream or in arbor, first.
- **Qt Quick** does the least work per frame and has the quietest still screen. It uses the most memory and missed 11% of frames under load.
- **Flutter** came second under load. It wakes 0.3 times a second on a still screen and is at the limit for the first frame. It adds Dart and a GTK 3 runner.

## The first app, measured

[#9](https://github.com/NahuelGenchi/arbor/issues/9) built the first Mission Control app, [`arbor-mission`](../prototype/mission/README.md): one read-only window in GTK 4, through gtk4-rs. The suite's `mission` section (`ONLY=mission bench/run.sh`) measures it with the method used for the toolkits above, on arbor state that the section makes in its temp folder from made-up folders. It measures three cases:

- **Nothing running:** 4 tries, 2 decisions waiting, a kept try and about 20 steps of history.
- **Drawn by cairo:** the same state, drawn by GTK's cairo renderer (`GSK_RENDERER=cairo`). It draws on the CPU and loads no GPU driver; GTK's default on this PC is its OpenGL renderer.
- **300 rooms:** the same state, plus 300 more tries with a room working in each. One more try runs a short command every 0.2 s, and each one records two steps, so the ledger keeps moving. Each room is a sealed cell under a memory limit, with a cgroup that the window reads. The rooms sleep, so what they use does not change. What changes on screen is the history and the try that keeps running commands.

The window was not changed to suit the method. `--bench still` is the window as it is. `--bench animate` moves its first decision the way the test program moves its card, and reads ten times as often as the window does: the ledger's new steps every 100 ms, and the rooms every second.

**Conditions.** Same machine as above. Each column is one run, from an ordinary process (a systemd user unit). The first two are from #9's run on 18 October 2026 at 13:02. The 300-room column is from [#78](https://github.com/NahuelGenchi/arbor/issues/78)'s run the same day at 14:17, after the window was changed to read only what new steps name; its run started at a load average of 0.31, and every row passed the section's checks. A run of #78 at 13:52 is not used: during its 300-room still minute another program's windows were drawing, and the section failed those rows. In #9's run: Every row passed the section's checks, and no key or mouse was used during the still minutes. The run started at a load average of 0.15. The cairo rows began right after the loaded animation before them, while the 1-minute load average was still falling: it was 7.0 at their first launch and 1.5 to 2.1 for their animation. No other program used a CPU thread during them, and the section now waits for the load to settle at that point too. The first frame is the median of 5 launches; every other figure is one sample.

| Measure (M: every number in the row) | Nothing running | Drawn by cairo | 300 rooms | Target |
| :--- | ---: | ---: | ---: | :--- |
| First frame, ms (M) | 174 (165–175) | 126 (126–127) | 200 (192–206) | at most 200 |
| Memory 2.5 s after the start, RSS / PSS, MiB (M) | 141 / 60 | 54 / 24 | 161 / 84 | |
| Memory after a minute of still screen, RSS / PSS, MiB (M) | 141 / 60 | 54 / 24 | 162 / 84 | at most 128 / 64 with 300 agents and every view |
| Still screen: frames shown a second (M) | 0 | 0 | 1.0 | 0 with nothing changing; at most 1 busy |
| Still screen: CPU, % of one core (M) | 0.00 | 0.00 | 0.33 | at most 0.1 with nothing changing; at most 1 busy |
| Still screen: wakeups a second (M) | 0.10 | 0.03 | 12.5 | at most 0.1 with nothing changing |
| Animation, machine idle: frames missed, % (M) | 0.42 | 0.42 | 0.17 | none |
| Animation, machine idle: slowest 1% of frame times, ms (M) | 16.7 | 16.7 | 16.7 | |
| Animation: work per frame, slowest 1%, ms (M) | 0.58 | 0.73 | 8.98 | at most 4 |
| Animation under load: frames missed, % (M) | 15.1 | 13.1 | 4.9 | at most 0.1 |
| Animation under load: slowest 1% of frame times, ms (M) | 33.3 | 33.3 | 33.4 | |
| Objects on the accessibility bus (M) | 173 | 173 | 371 | |
| The window's words found on AT-SPI (M) | 8 of 8 | 8 of 8 | 4 of 4 | every control |

What the table shows:

- **With nothing running, the window is still.** No frame in the minute, no CPU time counted, and one wakeup every 10 s, which is the target.
- **With 300 rooms, it is busy but calm: it meets the target.** It showed one frame a second while the ledger moved, and used 0.33% of a core against the 1% target. It woke 12.5 times a second: 9.0 on its main thread, which draws that frame, 2.4 on the threads that read, and 1.0 on GTK's own. In #9's run it used 2.37% of a core and woke 32.3 times a second (18.7 on the main thread, 9.5 on GLib's `gmain` and 3.0 reading), because at most once a second it read again all 305 tries and what changed in each, the 300 rooms and their cgroups and the outbox. [#78](https://github.com/NahuelGenchi/arbor/issues/78) changed three things. A reading now reads only what the ledger's new steps name: the worlds they name, the cells they start or end, and the outbox after a step about an effect. What rooms use, and the files they change in their tries, are read every 10 s with the full list of rooms, since no step records them. While the ledger keeps moving, the window stops watching it and reads it on each calm second instead, so its writes no longer wake `gmain` and the main thread; once a reading finds nothing new, it watches again. And arbor parsed each running cell's file one byte per system call; it now reads the file whole first, in the 10 s sample and in `arbor ps` alike.
- **Work per frame is 0.58 ms with nothing running**, within 4 ms. With 300 rooms, the slowest 1% took 8.98 ms, down from 17.5 ms in #9's run, but that is not the frame's own work: the probe counts the whole program's CPU time between two frames. In `--bench animate` the window reads the rooms every second, ten times as often as it does, so about one frame in 60 (est.) also holds a full room sample, more than the 1% this row looks at. The median was 0.27 ms and the mean 0.55 ms, and frames were not missed more often than with nothing running. This run does not separate the main thread's time from the rest.
- **The first frame** takes 174 ms with nothing running. With 300 rooms it takes 200 ms, at the target, and 201 ms in #9's run: the window's first reading still reads all 305 tries and 300 rooms, although it runs while GTK starts.
- **Frames missed, machine idle:** 0.25 to 0.42%, as with the test programs above. None of them meets "none".
- **Frames missed under load:** 4.9 to 15.1%, and 7.9 to 9.9% in the run before this one, of the same app (12:46). That is around the minimal client's 8.35% and far from 1 in 1,000. Keeping time and memory for the screen is [#10](https://github.com/NahuelGenchi/arbor/issues/10)'s work.
- **Accessibility.** The suite looked for the window's own words on the bus, and every one was there: the heading, both decisions' commands and the tries' names. With 300 rooms, the bus holds 371 objects, not thousands, because only 12 tries and 3 rooms in each are shown, and the rest fold into one line.
- **Cairo drew this window as well as the default renderer did.** Its first frame was 48 ms sooner and its work per frame was 0.73 ms against 0.58, with the same share of frames missed when idle. This window is small (960 × 640) and moves one line at a time. A larger window, a 4K panel or more motion would cost cairo more CPU time; that was not measured.

### The two points #8 left open

**Memory.** With GTK's default renderer, the window uses 141 MiB of RSS and 60 MiB of PSS with nothing running. With 300 rooms, after a minute, it uses 162 and 84 MiB (#78's run; 164 and 82 in #9's). That is over the RSS target in both states, and over the PSS target with 300 rooms. It did not grow over the minute in either state.

Most of that memory is the GPU driver. Drawn by cairo, which loads no driver, the same window uses 54 MiB of RSS and 24 MiB of PSS (M). With 300 rooms, the window's anonymous memory is 21 MiB larger, 59 MiB of PSS against 38 (M). Under cairo, that would make about 75 MiB of RSS (est.), within the target, but the 300-room state was not measured with cairo.

PSS also moved more than RSS from one day to the next. GTK 4's test program had 118 MiB of PSS at its start on 17 October and 60 MiB on 18 October, with the same RSS (M, both). RSS is the steadier comparison.

Which renderer Mission Control uses is not decided here. Cairo meets the memory target with this window. The GPU renderer may be needed for larger windows or more motion, and neither renderer was measured at that size.

**The bindings.** gtk4-rs adds nothing the suite can measure. The same run (18 October 2026 at 12:07, load average 0.40 at the start) measured the C test program and the same program written in Rust through gtk4-rs, call for call ([`bench/toolkits/gtk4rs/`](../bench/toolkits/README.md)). Every row passed the section's checks. Each figure is one sample, and the first frame is the median of 5 launches:

| Measure (M: every number in the row) | C | gtk4-rs |
| :--- | ---: | ---: |
| First frame, ms (M) | 175 (173–184) | 159 (158–175) |
| Memory 2.5 s after the start, RSS / PSS, MiB (M) | 140 / 60 | 141 / 61 |
| Memory after a minute of still screen, RSS / PSS, MiB (M) | 161 / 81 | 141 / 61 |
| Still screen: CPU, % of one core / wakeups a second (M) | 0.02 / 0.10 | 0.02 / 0.10 |
| Animation, machine idle: frames missed, % (M) | 0.25 | 0.33 |
| Animation: work per frame, slowest 1%, ms (M) | 0.95 | 0.83 |
| Animation under load: frames missed, % (M) | 14.3 | 10.3 |
| Objects on the accessibility bus (M) | 127 | 127 |

The C program's anonymous memory grew by 20 MiB over its still minute, and its RSS grew from 140 to 163 MiB on 17 October as well. The port's memory did not grow, and neither did Mission Control's. Why the C program grows was not found.

### The buttons, measured

Release and Refuse ([#82](https://github.com/NahuelGenchi/arbor/issues/82)) are the window's first input. The `mission` section times them in two of the states above, nothing running and 300 rooms. In each, a try asks for 6 effects, and the window answers them through its own buttons (`arbor-mission --bench answer`), releasing and refusing in turn. Each press is timed from the button's clicked signal, as a press sends it, to the compositor's presentation time of the frame that shows its result, read from GTK's frame timings. Each figure is the median of its presses, with their range. The numbers are from the run of 18 October 2026 at 21:02, from an ordinary process, on the build with #82, [#83](https://github.com/NahuelGenchi/arbor/issues/83) and [#84](https://github.com/NahuelGenchi/arbor/issues/84). The run started at a load average of 1.39, and was at 0.28 and 0.52 at these rows. Every row passed the section's checks.

| From the press to the frame that shows (M: every number in the row) | Nothing running | 300 rooms | Target |
| :--- | ---: | ---: | :--- |
| Release or Refuse: the confirmation in place, ms (M) | 24 (21–30) | 25 (20–42) | at most 33, two refreshes |
| Release now: the receipt, once `arbor release` has run, ms (M) | 39 (32–41) | 47 (35–51) | |
| Refuse now: the receipt, once `arbor refuse` has run, ms (M) | 42 (38–44) | 45 (44–52) | |

- **The window answers a press within two refreshes.** The confirmation is the window's own answer, with no command to wait for. Its median is 24 ms with nothing running and 25 ms with 300 rooms, within the 33 ms of the target. One press of 12 took 42 ms, with 300 rooms.
- **A receipt waits for arbor.** Release now and Refuse now run `arbor release` or `arbor refuse` and show the receipt when the command ends: 39 to 47 ms. The receipt is the result, not the first answer to the press. That answer is the line saying it is releasing, which was not timed.

### Not measured yet

- **Input latency from the device**, and its 99th percentile. The presses above start at GTK's clicked signal, so the path from the keyboard or mouse to GTK is not in them, and 6 presses a state are too few for the slowest 1%.
- **Memory with every view opened** waits for the other views.
- **No growth over an hour of updates:** the longest window measured here is a minute.
- **120 Hz:** this PC's panel runs at 60 Hz.
- **The 300 rooms drawn by cairo.**
- **Rooms that work:** the 300 rooms here sleep, because 300 working rooms would make the machine too busy for the still-screen rows to count. What a room uses is read every 10 s, so rooms that work would add at most one update every 10 s to the still screen (est.).

## Not decided here

Keeping the screen ahead of agents with reserved time and memory ([#10](https://github.com/NahuelGenchi/arbor/issues/10)), the code of the views after the first window ([#11](https://github.com/NahuelGenchi/arbor/issues/11), [#12](https://github.com/NahuelGenchi/arbor/issues/12)), arbor's own compositor ([#13](https://github.com/NahuelGenchi/arbor/issues/13)), and a visual brand. The wireframes borrow the README's paper, ink and typefaces so the pictures read as one family; that is not a chosen look.

## Open questions

These are also listed in [issue #6](https://github.com/NahuelGenchi/arbor/issues/6), for the issues that follow.

1. **What is a task, in the core?** The planned objects have no task. This brief groups everything under the key a person handed out, with tries working under keys narrowed from it. That needs each key to name the key it came from, or the core to give each task an id.
2. **Who wrote the words on a line?** A task's title and a decision's summary can come from the person's request or from an agent. Agent text can be wrong, or planted by a page the agent read, so it is shown second and marked. Should the core keep the person's own request with the key?
3. **Can it be undone?** The decision needs *can be undone* as a fact from the outbox item, not as the agent's word. Should every outbox item say whether, and how, its effect can be undone?
4. **One queue for everything that needs a person?** A wider key or a raised budget needs a yes too. Should those be outbox items, so that Needs you reads one queue with one set of rules and receipts?
5. **What does `arbor watch` send?** Summaries per task or every ledger entry, batching per subscriber, how often a room's processor and memory are sampled, and how a client catches up after a gap by sequence number. The answer decides how much the screen has to fold.
6. **Where is energy?** The README and [#12](https://github.com/NahuelGenchi/arbor/issues/12) count energy as a budget; the budget object planned for the core has money, time, memory and attention. Energy can join when it is measured per cell (M5).
7. **How is attention counted?** In asks, as proposed here, or in minutes; who sets the limit; and what agents do once it is spent.
8. **How far back can history show?** Scrubbing can show the list of steps as it was, or a try itself as it was at any step. The second needs the core to keep every version of a world.
9. **More than one person, or away from the desk?** A team that shares decisions needs to say who may release what. A decision sent to a phone is itself a message leaving the machine, so it would go through the outbox too.
10. **What is it called?** "Mission Control" is also the name of a macOS feature. A final name is needed before anything ships.
11. **How is this tested with people before Release and Refuse are built?** For example: five people who don't write code, each asked to find what needs them in the first app ([#9](https://github.com/NahuelGenchi/arbor/issues/9)) and to say what Release will do.

## How the wireframes are made

The pictures are HTML pages in [`images/source/`](images/source/) (`mission-*.html`), drawn with the same colours and typefaces as the README's pictures and rendered by [`render.sh`](images/source/render.sh) in a light and a dark version (`./render.sh mission-now` renders one). Every name, number and message in them is made up.
