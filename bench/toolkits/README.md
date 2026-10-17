# The toolkits of issue #8

The same small Mission Control program, written once in each candidate toolkit for the first Mission Control app
([docs/INTERFACE.md](../../docs/INTERFACE.md)), so that the suite's `toolkits` section can measure them all the same
way: `ONLY=toolkits bench/run.sh` ([../README.md](../README.md)). The data is made up; nothing here talks to arbor.

| Folder | Toolkit | Language | Licence |
| :--- | :--- | :--- | :--- |
| `gtk4/` | GTK 4.22.4, Ubuntu 26.04's packages | C | LGPL-2.1-or-later |
| `qtquick/` | Qt Quick 6.10.2, Ubuntu 26.04's packages | C++ and QML | LGPL-3.0 or GPL-2.0, or Qt's commercial licence |
| `slint/` | Slint 1.18.1 | Rust | GPL-3.0-only, or Slint's royalty-free or commercial licence |
| `iced/` | iced 0.14.0 | Rust | MIT |

The licences are the packages' own: Debian's `copyright` files for GTK 4 and Qt, the crates' `license` fields for
Slint and iced.

## The program

One window, 960×640, titled `arbor bench: <toolkit>`, with a #fbfaf7 background, 14 px text and 16 px margins:

- a heading, `3 things need you`, 22 px and bold;
- a card 64 px high in a slot of 72 px, `Payments, moved to version 2: waiting for you. Can be undone.`, on #fff3dc
  with 8 px corners;
- a list of 300 rows of 32 px, in the toolkit's own scrolling list: a dot of 10 px (green, amber or grey), a title
  (`Help pages 1`, semibold, 200 px), a count (`37 of 153 pages translated`, 320 px), the agents (`8 agents`, 120 px)
  and a time (`until 9:00`), the last two fainter.

Row *i* starts at *done* = 37*i* mod *total* of *total* = 100 + 53*i* mod 900. Tick *k* changes the 30 rows whose
number ends in *k* mod 10: *done* grows by 1 + (*i* + *k*) mod 3, up to *total*. Each program takes one argument:

- `still`: a tick every 100 ms from 3 s, 20 in all; at 5 s the card arrives, from 40 px higher and transparent to its
  place and opaque, in 200 ms (ease out); after that nothing changes, so a toolkit has nothing to draw.
- `animate`: from 100 ms, the card rises 40 px while it fades to 25 % and comes back, 200 ms each way, without end,
  and a tick comes every 100 ms.

How each toolkit does it:

| | GTK 4 | Qt Quick | Slint | iced |
| :--- | :--- | :--- | :--- | :--- |
| The list | `GtkListView`, which makes widgets for the rows on screen | `ListView`, which makes delegates for the rows on screen | the standard widgets' `ListView`, which makes the rows on screen | a `column` of all 300 rows in a `scrollable`: iced 0.14 has no list that makes only the rows on screen |
| The card's motion | a tick callback on the window's frame clock, in the main thread | Animators, which run on the scene graph's render thread | `animate` on its properties, in the main thread | `Animation` with a frame subscription; the whole view is built again for each frame, in the main thread |
| Drawing, on this PC | GSK's OpenGL renderer: GTK tries Vulkan first and turns it down here ("Not using Vulkan: no dmabuf support") | the scene graph on the OpenGL backend of Qt's RHI | FemtoVG on OpenGL | wgpu on Vulkan |
| Accessibility | GTK's own AT-SPI support | Qt's AT-SPI bridge; the words are Qt Quick Controls' `Label`, see below | AccessKit, which speaks AT-SPI | none: iced 0.14 has no accessibility |

GTK 4 counts its title bar in its 960×640; Qt Quick, Slint and iced draw theirs around it, 962×680 and 960×675 on
GNOME, as each frame row's context shows. The drawing row was read on 17 October 2026 on this PC's Radeon R9 380 (Mesa
26.0.8): from GTK's own output with `GSK_DEBUG=renderer` and Qt's with `QSG_INFO=1`, and for Slint and iced from the
calls they make under gdb (Slint presents each frame with `eglSwapBuffers`; iced makes a Vulkan device and a Vulkan
Wayland surface, and no EGL window).

## Building

`build.sh OUT [gtk4 qtquick slint iced]` builds the programs into `OUT/<toolkit>/`, with `OUT/<toolkit>/bin` pointing
at each, and prints `built <toolkit>` or `failed <toolkit>: <why>`. The section runs it with its temp folder, or
`TOOLKITS_PREBUILT` names a folder it built them in before. It writes only in `OUT`; cargo's objects, about a gigabyte
a program, are removed once each program is built.

| Toolkit | To build | To run |
| :--- | :--- | :--- |
| GTK 4 | gcc, pkg-config and `libgtk-4-dev` | GTK 4 |
| Qt Quick | cmake (with ninja when there is one), g++, `qt6-base-dev` and `qt6-declarative-dev`, which on Ubuntu 26.04 brings the QML modules the program imports | `qt6-wayland`, for a Wayland window |
| Slint, iced | Rust 1.99.0, the prototype's (`rust-toolchain.toml`), and the crates in each `Cargo.lock`, downloaded once with `cargo fetch` in `slint/` and in `iced/`, since the build is offline; Slint also links fontconfig, from `libfontconfig-dev` | Mesa's OpenGL (Slint) or Vulkan (iced) driver, which both load at run time, as they do Wayland's libraries |

## Accessibility

The section's accessibility check starts each program in a D-Bus session of its own, turns accessibility on there, and
walks the program's objects on the accessibility bus with libatspi ([../README.md](../README.md)). With Qt 6.10, a
plain `Text` in Qt Quick does not reach the bus: the first version of `qtquick/Main.qml`, with `Text` for its words,
showed 3 objects (the application, the window and the scroll bar), and so did the same program with
`QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1`. The same program with each `Text` given `Accessible.role: Accessible.StaticText`
and `Accessible.name: text`, and the same program with Qt Quick Controls' `Label` in place of `Text`, both showed 109:
the heading, the card and the words of the 26 rows the list had made. The program uses `Label`. These checks were run
on 17 October 2026, on scratch builds of the three versions; they are not rows of the suite.
