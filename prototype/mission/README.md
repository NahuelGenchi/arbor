# Mission Control

The first Mission Control app ([#9](https://github.com/NahuelGenchi/arbor/issues/9)): one window on today's desktop that shows what arbor is doing, in the plain words of the design brief ([`docs/INTERFACE.md`](../../docs/INTERFACE.md)). It is written in Rust with GTK 4, through the gtk4-rs bindings, the toolkit [#8](https://github.com/NahuelGenchi/arbor/issues/8) chose. So far it only reads: Release and Refuse from the window are the next piece, and until then each decision names the commands that answer it.

![Mission Control on made-up worlds: two decisions wait, four rooms work in four of eight tries, and the last steps of history on the right](../../docs/images/mission-app.png)

The window shows, from arbor's own state folder:

- **Needs you**: the effects waiting in the outbox for a person, each with its command, where it runs, what it says it costs and why, in the agent's own words and marked as not checked.
- **Tries**: every world, working ones first, with the rooms working in each (the cells running there), their command, key, memory and share of a processor, and what changed in words (*3 files changed*). Past 12 tries, and past 3 rooms in a try, the rest fold into one line.
- **History**: the ledger's last 8 steps, newest first. A step that arrives later does not push the list down under the reader's eyes: a line says *There are 2 newer steps*, and a button shows them.

## Build and run

It needs GTK 4.14 or later with its development files (`sudo apt install libgtk-4-dev` on Ubuntu 24.04 and later) and the Rust toolchain pinned in [`../rust-toolchain.toml`](../rust-toolchain.toml). In `prototype/`:

```sh
cargo build --release -p mission     # target/release/arbor-mission
cargo install --locked --path mission  # or install it into ~/.cargo/bin
```

`arbor-mission` opens the window on the state of the `arbor` command: `~/.local/share/arbor`, or `$ARBOR_HOME`. It changes nothing there and starts nothing. Ctrl+W or Ctrl+Q closes it.

To try it without your own folders, `mission/demo.sh` makes a home with four small projects and an arbor state folder in a temp folder: eight tries, four rooms working (each holds some memory and waits), two decisions waiting and a kept try. Then it opens the window there. Closing the window stops the rooms and removes the folder. It needs bubblewrap 0.10 or later and, for the rooms' memory and processor, a systemd user session; it starts `arbor serve` if it is not running, and stops it again.

```sh
cargo build --release -p arbor -p mission --bins --example shot
mission/demo.sh                              # the window
mission/demo.sh shot demo.png dark 960x800   # or draw it into a picture, as the one above was
```

## How it keeps still

- **It reads only when arbor writes, and only what changed.** The window watches the ledger with inotify (GLib's file monitor), since every change to worlds, cells and the outbox is a step there, and polls nothing. It reads at most once a second, on a thread of GLib's pool, and then changes only the words that changed. The ledger is read from where the last reading stopped, and a reading reads again only what its new steps name: those worlds, the cells they start or end, and the outbox after a step about an effect. While the ledger keeps moving, the window stops watching it and reads it on each calm second instead, so its writes wake nothing; once a reading finds nothing new, it watches again.
- **Rooms are sampled, not followed.** While rooms with a cgroup run, their memory and processor, the list of running cells and the files changed in the tries they work in are read again every 10 s; with none, nothing is.
- **A hidden window reads nothing.** When the desktop says the window is not seen (GTK's `suspended`, a minimised window for one), readings wait until it is shown again, and then one reading catches up.
- **Motion is short and rare.** A new line arrives from 12 px higher and fades in, in 200 ms, easing out, at most 5 at once; more than that just appear. Only the line's drawing moves, never the layout. Under the desktop's reduced motion, or with its animations off, nothing moves.

## Accessibility and look

- Each line reads as one sentence on the accessibility bus, for example *Decision waiting since 14:05: run git push origin fix, as you, with the network, in ~/projects/shop, asked from try shop. In its own words, not checked: “publish the fix for rounding the total”, says it costs $0.40.* The heading and the sections are headings there, and each line can be reached with the keyboard.
- Only what needs a person is announced, politely: *A decision is waiting: git push origin fix*, or *2 decisions are waiting*, when new ones arrive. Routine progress is not.
- A state is never shown by colour alone: each has a word and a mark with a shape of its own.
- The colours are the brief's, which meet WCAG 2.2 AA in both themes. With GTK 4.20 or later the window follows the desktop's light or dark setting and its high contrast; before 4.20 it is light. Sizes follow the desktop's text size, and below 820 px the two columns become one.

## Measured

`ONLY=mission bench/run.sh` measures the window as the suite measured #8's test programs, on arbor state that it makes in its temp folder ([`bench/README.md`](../../bench/README.md)). The section is about fifteen minutes long, and the PC has to be left alone while it runs. The measurements below come from two runs on 18 October 2026, from an ordinary process: nothing running from #9's run at 13:02, and 300 rooms from #78's run at 14:17, after the window was changed to read only what new steps name, on the PC of [the toolkits measured](../../docs/INTERFACE.md#the-toolkits-measured). Each figure is one sample, and each first frame is the median of 5 launches. [*The first app, measured*](../../docs/INTERFACE.md#the-first-app-measured) has every row, the conditions, and what the numbers mean.

| Measure (M: every number in the row) | Nothing running | 300 rooms | Target |
| :--- | ---: | ---: | :--- |
| First frame, ms (M) | 174 | 200 | at most 200 |
| Memory after a minute, RSS / PSS, MiB (M) | 141 / 60 | 162 / 84 | at most 128 / 64 |
| Still screen: frames a second / CPU, % of a core / wakeups a second (M) | 0 / 0.00 / 0.10 | 1.0 / 0.33 / 12.5 | 0 / 0.1 / 0.1 still; 1 / 1 busy |
| Animation: frames missed idle / under load, % (M) | 0.42 / 15.1 | 0.17 / 4.9 | none / 0.1 |
| Work per frame, slowest 1%, ms (M) | 0.58 | 8.98, with a room sample every second | at most 4 |

With nothing running, the window meets the still-screen targets. With 300 rooms, it shows one frame a second and uses 0.33% of a core, within the "busy but calm" target of 1; before it read only what new steps name, it used 2.37% (M, #9's run). Most of the memory is the GPU driver. Drawn with GTK's cairo renderer (`GSK_RENDERER=cairo arbor-mission`), the same window with nothing running uses 54 MiB of RSS and 24 MiB of PSS (M).

## For developers

| Path | What |
| :--- | :--- |
| `src/state.rs` | reading arbor's state: everything the first time, then only what the ledger's new steps name (worlds and their changes, the cells started or ended, the outbox's waiting effects), and every running cell with what each uses at the 10 s sample |
| `src/words.rs` | the plain words of every line, and what each line says on the accessibility bus |
| `src/ui.rs` | the window: its sections, updating only the lines that changed, the file monitor, which rests while the ledger moves, and the calm second, the room sampling, the announcements, and the pictures `examples/shot.rs` draws |
| `src/shift.rs` | the widget that moves and fades a line as it is drawn |
| `src/mission.css` | the look: the brief's colours, light, dark and high contrast |
| `demo.sh` | made-up worlds, rooms and decisions, and the window on them |

`arbor-mission --bench still` and `--bench animate` are for the suite's `mission` section ([`bench/README.md`](../../bench/README.md)): `animate` moves the first decision without end, as #8's test program moves its card, and reads ten times as often as the window does: the ledger every 100 ms and the rooms every second. `../check.sh` lints and tests the app with arbor when GTK's development files are there.
