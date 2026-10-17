# Mission Control: design brief

[Back to README](../README.md) · [Issue #6](https://github.com/NahuelGenchi/arbor/issues/6) · [M7 — Interface](https://github.com/NahuelGenchi/arbor/milestone/8)

> [!NOTE]
> **No interface exists yet.** This brief decides what Mission Control shows and how a person directs agents through it, before a toolkit is chosen ([#8](https://github.com/NahuelGenchi/arbor/issues/8)) or any code is written ([#9](https://github.com/NahuelGenchi/arbor/issues/9)). The pictures are wireframes with made-up data, and its targets are for later issues to measure: none of them is a result.

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

| arbor's term | on screen | what a person needs to know |
| :--- | :--- | :--- |
| world | a **try** | a copy of your files that an agent works on, kept or thrown away whole |
| cell | a **room** | where an agent's work runs, sealed off from everything else |
| key | a **key** | what an agent may open, until when, and how much it may spend |
| outbox item | a **decision**, while it waits | something that will leave the machine: a deployment, an email, a payment |
| ledger | **history** | every step every agent took, in order |
| budget | a **budget** | money, time, memory and your attention |
| *(none yet)* | a **task** | what you asked for; it can hold many tries, rooms and agents |

## The views

Each view names the typed objects it reads: worlds (M2), cells and keys (M3), and the outbox, ledger and budgets (M6). The core is planned to publish them over a Unix socket (`arbor watch`) that Mission Control subscribes to, with no polling. The fields below are the ones planned for the core in M2–M6; where a view needs more, it is listed under [open questions](#open-questions).

### 1. Now

The first screen, and the one most people will use most. Top to bottom:

- **One sentence** that answers the question the screen exists for: *3 things need you*, or *Nothing needs you*, with how many agents are working on how many tasks.
- **Needs you**: decisions waiting, tasks stopped by a budget, and agents asking for a wider key, oldest first. Each is one line that opens its decision.
- **Working**: one line per task, with how far it has got in words that change rarely (*180 of 240 pages translated*, *trying 6 ways, 2 thrown away*), how many agents, and until when.
- **Done today**: one line with a count, which opens History.
- **Budgets**: today's money, memory and attention, spent against the limit, and when the last task ends.

**Reads:** outbox items (waiting, with their cost); worlds (open, kept or thrown away, to count tries); cells (running or idle, to count agents, and memory used against limits); keys (to group rooms into tasks, and their expiry); budgets (money, time, memory, attention).

### 2. Decision

One outbox item and everything needed to answer it, in one place:

- **What will leave the machine**, where it goes, what it costs, and whether it can be undone: facts from the outbox item.
- **The change it carries**, if any: the try and how many files it changes, which opens Changes.
- **What checked it**: the checks and their results, from the ledger.
- **What it has spent** of its budget.
- **In the agent's own words**: its summary, shown last and marked as not checked.
- **The steps behind it**: the ledger entries that produced it, oldest first, with a way to the whole history.

**Release** and **Refuse** answer it in one gesture with all of this in view. An effect that cannot be undone, or that costs more than a limit the person sets, also asks for a second confirmation, such as a touch of a security key. A release goes through the outbox's own rules and leaves its receipt in the ledger; Mission Control has no other way to send anything off the machine ([#12](https://github.com/NahuelGenchi/arbor/issues/12)).

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

**Reads:** a world (its name, the folder it was copied from, the files changed, and whether it is open, kept or thrown away). The outline needs the outline-first code objects of M2; until they exist, the view shows the list of files and their text changes, as `arbor diff` gives them ([#14](https://github.com/NahuelGenchi/arbor/issues/14)).

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

These are targets for the issues that build and measure Mission Control. **None of them has been measured.** Each issue records its numbers, marked **M**, with the machine and its conditions.

- **Missed frames.** *Target:* none with the machine idle, and at most 1 in 1,000 under load (300 agents in cells plus a memory spike), at 60 Hz and at 120 Hz. Checked by the frame-pacing rows of [#7](https://github.com/NahuelGenchi/arbor/issues/7), then by [#9](https://github.com/NahuelGenchi/arbor/issues/9) to [#12](https://github.com/NahuelGenchi/arbor/issues/12) with 300 agents.
- **Work per frame.** *Target:* the app's own work for a frame takes at most 4 ms for 99% of frames: a quarter of a frame at 60 Hz (16.7 ms) and half of one at 120 Hz (8.3 ms), so the compositor and the GPU keep the rest. Checked by the test program of [#8](https://github.com/NahuelGenchi/arbor/issues/8), then by [#9](https://github.com/NahuelGenchi/arbor/issues/9).
- **Input latency.** *Target:* an input is answered in the next frame. From the input event to the frame that shows its result takes at most 2 refreshes for 99% of inputs (33 ms at 60 Hz, 17 ms at 120 Hz), idle and under load. Checked with the presentation timing of [#7](https://github.com/NahuelGenchi/arbor/issues/7), by [#9](https://github.com/NahuelGenchi/arbor/issues/9) and [#10](https://github.com/NahuelGenchi/arbor/issues/10). Input to photon, from the device to the panel, needs a camera or a hardware tester and is out of scope in #7.
- **Still screen.** *Target:* with nothing changing, 0 frames drawn, at most one wakeup every 10 s, and at most 0.1% of a core over 60 s. Today, gnome-shell alone uses 2.1% of a core on a still screen (M, [OPTIMIZATION.md §1](../OPTIMIZATION.md#1-ram-cpu-gpu)). Checked by the still-screen row of [#7](https://github.com/NahuelGenchi/arbor/issues/7), then by [#9](https://github.com/NahuelGenchi/arbor/issues/9).
- **Busy but calm.** *Target:* with 300 agents working and nothing needing a person, at most 1 frame a second and 1% of a core. Checked with #7's method by [#9](https://github.com/NahuelGenchi/arbor/issues/9), [#11](https://github.com/NahuelGenchi/arbor/issues/11) and [#12](https://github.com/NahuelGenchi/arbor/issues/12).
- **Memory.** *Target:* at most 128 MiB RSS and 64 MiB PSS with 300 agents and every view opened once, under a tenth of the Electron UI measured today (1,354 MiB RSS, M, [OPTIMIZATION.md §1](../OPTIMIZATION.md#1-ram-cpu-gpu)); within 10% of the same app with 3 agents; and no growth over an hour of updates. Checked by [#8](https://github.com/NahuelGenchi/arbor/issues/8) and [#9](https://github.com/NahuelGenchi/arbor/issues/9).
- **First frame.** *Target:* at most 200 ms from launch to the first frame that shows real state, with warm caches. Checked by [#8](https://github.com/NahuelGenchi/arbor/issues/8).

## Not decided here

The rendering stack ([#8](https://github.com/NahuelGenchi/arbor/issues/8)), the app itself ([#9](https://github.com/NahuelGenchi/arbor/issues/9)), keeping the screen ahead of agents with reserved time and memory ([#10](https://github.com/NahuelGenchi/arbor/issues/10)), the code of the views ([#11](https://github.com/NahuelGenchi/arbor/issues/11), [#12](https://github.com/NahuelGenchi/arbor/issues/12)), arbor's own compositor ([#13](https://github.com/NahuelGenchi/arbor/issues/13)), and a visual brand. The wireframes borrow the README's paper, ink and typefaces so the pictures read as one family; that is not a chosen look.

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
11. **How is this tested with people before [#9](https://github.com/NahuelGenchi/arbor/issues/9)?** For example: five people who don't write code, each asked to find what needs them and to say what Release will do.

## How the wireframes are made

The pictures are HTML pages in [`images/source/`](images/source/) (`mission-*.html`), drawn with the same colours and typefaces as the README's pictures and rendered by [`render.sh`](images/source/render.sh) in a light and a dark version (`./render.sh mission-now` renders one). Every name, number and message in them is made up.
