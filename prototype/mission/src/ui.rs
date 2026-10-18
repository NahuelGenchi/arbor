//! The window: one column of what needs you and the tries with their rooms, and a side column with the last steps of
//! history, which drops below on a narrow window.
//!
//! How it stays quiet (docs/INTERFACE.md, *How an update reaches the screen*): it watches arbor's ledger with inotify,
//! since every step that changes a world, a cell or the outbox is recorded there, and sleeps until the ledger moves.
//! A change starts a reading at once, and the next one waits until a second has passed, so a busy ledger is read at
//! most once a second. A reading runs off the main thread; the window then changes only the lines whose words
//! changed, and GTK draws them on the next frame. What rooms use changes without a step, so while rooms run and the
//! window is shown, it reads them again every 10 s. A still screen has no timer running.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{gdk, gio, glib};

use crate::shift::Shift;
use crate::state::{self, Kind, Reader, Snapshot};
use crate::words::{self, Line, Mark};

/// How many steps of history the side column shows.
const STEPS: usize = 8;
/// How many tries show before the rest fold into one line, and how many rooms a try shows.
const TRIES_SHOWN: usize = 12;
const ROOMS_SHOWN: usize = 3;
/// At most one reading a second.
const CALM: Duration = Duration::from_secs(1);
/// Seconds between readings of what the rooms use, while they run and the window is shown.
const SAMPLE_S: u32 = 10;
/// Below this width the side column drops below the first.
const NARROW: i32 = 820;
/// The side column's lines wrap at this many characters.
const SIDE_CHARS: i32 = 34;
/// More new lines than this at once appear without motion: nobody can follow them all.
const MOVING: usize = 5;

const CSS: &str = include_str!("mission.css");

/// The suite's modes (`--bench`), so that it can measure the window as the test programs of issue #8 are measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bench {
    /// The window as it is: it shows arbor's state and sleeps.
    Still,
    /// The first line moves without end, and arbor's state is read every 100 ms.
    Animate,
}

/// How the window runs.
#[derive(Debug, Clone, Default)]
pub struct Options {
    pub bench: Option<Bench>,
    /// Draw the window into this PNG file once it shows arbor's state, then quit.
    pub png: Option<PathBuf>,
    /// Dark (true) or light (false), instead of the desktop's setting.
    pub dark: Option<bool>,
    pub size: Option<(i32, i32)>,
}

pub fn run(options: Options) -> glib::ExitCode {
    // The first reading of arbor's state runs while GTK starts, so the first frame shows it without waiting for it.
    let first = std::thread::spawn(|| {
        let mut reader = Reader::new(STEPS);
        let snap = reader.read(Kind::All);
        (reader, snap)
    });
    let first = RefCell::new(Some(first));
    // The window's state, held while the app runs: its signal handlers hold it only weakly.
    let held = RefCell::new(None);
    let app = gtk::Application::builder()
        .application_id("org.arbor.Mission")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.set_accels_for_action("window.close", &["<Control>w", "<Control>q"]);
    app.connect_activate(move |app| {
        let first = first.take().and_then(|t| t.join().ok());
        let ui = Ui::build(app, options.clone());
        ui.start(first);
        held.replace(Some(ui));
    });
    // The arguments are arbor-mission's own, not GTK's.
    let argv0: Vec<String> = std::env::args().take(1).collect();
    app.run_with_args(&argv0)
}

/// One list of lines: needs you, tries or steps.
struct Section {
    root: gtk::Box,
    list: gtk::ListBox,
    rows: RefCell<Vec<Row>>,
    /// The side column's lines wrap at this many characters, so that it keeps its width; -1 for no limit.
    max_chars: i32,
}

/// The widgets of one line, and the words they show.
struct Row {
    line: Line,
    row: gtk::ListBoxRow,
    shift: Shift,
    mark: gtk::Widget,
    title: gtk::Label,
    command: gtk::Label,
    detail: gtk::Label,
    aside: gtk::Label,
    note: gtk::Label,
}

struct Ui {
    window: gtk::ApplicationWindow,
    options: Options,
    heading: gtk::Label,
    lead: gtk::Label,
    problems: gtk::Label,
    columns: gtk::Box,
    side: gtk::Box,
    needs: Section,
    tries: Section,
    steps: Section,
    newer: gtk::Box,
    newer_label: gtk::Label,
    /// The newest step the history shows: the list holds still until asked for newer ones.
    shown_seq: Cell<u64>,
    first: Cell<bool>,
    latest: RefCell<Snapshot>,
    reader: RefCell<Option<Reader>>,
    pending: Cell<Option<Kind>>,
    last_read: Cell<Option<Instant>>,
    calm: RefCell<Option<glib::SourceId>>,
    sample: RefCell<Option<glib::SourceId>>,
    monitor: RefCell<Option<gio::FileMonitor>>,
}

fn label(classes: &[&str]) -> gtk::Label {
    let l = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .css_classes(classes.iter().map(|c| c.to_string()).collect::<Vec<_>>())
        .build();
    l.set_natural_wrap_mode(gtk::NaturalWrapMode::Word);
    l
}

impl Section {
    fn new(title: &str, aside: Option<&str>, max_chars: i32) -> Section {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let t = gtk::Label::builder()
            .label(title)
            .xalign(0.0)
            .hexpand(true)
            .css_classes(["section"])
            .accessible_role(gtk::AccessibleRole::Heading)
            .build();
        t.update_property(&[gtk::accessible::Property::Level(2)]);
        head.append(&t);
        if let Some(a) = aside {
            let l = label(&["section"]);
            l.set_label(a);
            l.set_wrap(false);
            head.append(&l);
        }
        root.append(&head);
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["lines"])
            .build();
        list.update_property(&[gtk::accessible::Property::Label(title)]);
        root.append(&list);
        Section {
            root,
            list,
            rows: RefCell::new(Vec::new()),
            max_chars,
        }
    }

    /// Shows `lines` in order: lines that stayed keep their widgets and change only the words that changed, new
    /// ones arrive (with motion when `arrive`), and lines that went are removed. Returns the keys that are new.
    fn show(&self, lines: Vec<Line>, arrive: bool) -> Vec<String> {
        let mut rows = self.rows.borrow_mut();
        let keys: HashSet<&str> = lines.iter().map(|l| l.key.as_str()).collect();
        rows.retain(|r| {
            let keep = keys.contains(r.line.key.as_str());
            if !keep {
                self.list.remove(&r.row);
            }
            keep
        });
        // Many changes at once: past a handful, lines appear in place without moving.
        let fresh = lines
            .iter()
            .filter(|l| !rows.iter().any(|r| r.line.key == l.key))
            .count();
        let arrive = arrive && fresh <= MOVING;
        let mut new = Vec::new();
        for (i, line) in lines.into_iter().enumerate() {
            match rows.iter().position(|r| r.line.key == line.key) {
                Some(p) if p == i => rows[i].update(line, false),
                Some(p) => {
                    // Its band changed: it moves, as a whole.
                    let mut r = rows.remove(p);
                    self.list.remove(&r.row);
                    self.list.insert(&r.row, i as i32);
                    r.update(line, false);
                    rows.insert(i, r);
                }
                None => {
                    new.push(line.key.clone());
                    let r = Row::new(line, self.max_chars);
                    self.list.insert(&r.row, i as i32);
                    if arrive {
                        r.shift.arrive();
                    }
                    rows.insert(i, r);
                }
            }
        }
        self.root.set_visible(!rows.is_empty());
        new
    }
}

impl Row {
    fn new(line: Line, max_chars: i32) -> Row {
        let mark: gtk::Widget = match &line.badge {
            Some(word) => {
                let pill = gtk::Box::builder()
                    .orientation(gtk::Orientation::Horizontal)
                    .spacing(6)
                    .css_classes(["pill", "wait"])
                    .valign(gtk::Align::Start)
                    .build();
                let dot = gtk::Box::builder()
                    .css_classes(["dot"])
                    .valign(gtk::Align::Center)
                    .build();
                pill.append(&dot);
                let w = gtk::Label::new(Some(word));
                pill.append(&w);
                pill.upcast()
            }
            None => gtk::Box::builder()
                .css_classes(["mark"])
                .valign(gtk::Align::Start)
                .halign(gtk::Align::Center)
                .build()
                .upcast(),
        };
        let title = label(&["title"]);
        title.set_wrap_mode(gtk::pango::WrapMode::Word);
        title.set_width_chars(12);
        // A command in a step: one line, cut at its end; the whole of it is in the tooltip.
        let command = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["detail", "code"])
            .visible(false)
            .build();
        let detail = label(&["detail"]);
        let aside = label(&["aside"]);
        aside.set_xalign(1.0);
        aside.set_valign(gtk::Align::Start);
        aside.set_wrap_mode(gtk::pango::WrapMode::Word);
        let note = label(&["note"]);
        for l in [&title, &command, &detail, &note, &aside] {
            l.set_max_width_chars(max_chars);
        }
        title.set_valign(gtk::Align::Start);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        title.set_hexpand(true);
        top.append(&title);
        top.append(&aside);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        text.set_hexpand(true);
        text.append(&top);
        text.append(&command);
        text.append(&detail);
        text.append(&note);
        let body = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let slot = gtk::Box::builder()
            .css_classes(["slot"])
            .valign(gtk::Align::Start)
            .visible(line.mark != Mark::None || line.badge.is_some())
            .build();
        slot.append(&mark);
        body.append(&slot);
        body.append(&text);
        let shift = Shift::new(&body);
        let row = gtk::ListBoxRow::builder()
            .activatable(false)
            .selectable(false)
            .child(&shift)
            .build();
        if line.key.starts_with("cell ") || line.key.starts_with("more ") {
            row.add_css_class("room");
        }
        let mut r = Row {
            line: line.clone(),
            row,
            shift,
            mark,
            title,
            command,
            detail,
            aside,
            note,
        };
        r.update(line, true);
        r
    }

    /// Sets the words that changed, or all of them on a new row.
    fn update(&mut self, line: Line, all: bool) {
        let old = &self.line;
        if all || old.title != line.title || old.code != line.code {
            self.title.set_label(&line.title);
            if line.code {
                self.title.add_css_class("code");
            } else {
                self.title.remove_css_class("code");
            }
        }
        if all || old.command != line.command {
            self.command
                .set_label(line.command.as_deref().unwrap_or(""));
            self.command.set_visible(line.command.is_some());
        }
        if all || old.detail != line.detail {
            self.detail.set_label(&line.detail);
            self.detail.set_visible(!line.detail.is_empty());
        }
        if all || old.aside != line.aside {
            self.aside.set_label(&line.aside);
            self.aside.set_visible(!line.aside.is_empty());
        }
        if all || old.note != line.note {
            self.note.set_label(line.note.as_deref().unwrap_or(""));
            self.note.set_visible(line.note.is_some());
        }
        if (all || old.mark != line.mark) && line.badge.is_none() {
            for c in ["work", "idle", "kept", "none"] {
                self.mark.remove_css_class(c);
            }
            self.mark.add_css_class(match line.mark {
                Mark::Work => "work",
                Mark::Idle => "idle",
                Mark::Kept => "kept",
                Mark::Wait | Mark::None => "none",
            });
        }
        if all || old.tip != line.tip {
            self.row.set_tooltip_text(Some(&line.tip));
        }
        if all || old.said != line.said {
            self.row
                .update_property(&[gtk::accessible::Property::Label(&line.said)]);
        }
        self.line = line;
    }
}

impl Ui {
    fn build(app: &gtk::Application, options: Options) -> Rc<Ui> {
        let provider = gtk::CssProvider::new();
        follow_desktop(&provider);
        provider.load_from_string(CSS);
        if let Some(display) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
        if let Some(dark) = options.dark {
            set_scheme(dark);
        }
        let (w, h) = options.size.unwrap_or((960, 640));
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title("Mission Control")
            .default_width(w)
            .default_height(h)
            .css_classes(["mission"])
            .build();

        let heading = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .css_classes(["heading"])
            .accessible_role(gtk::AccessibleRole::Heading)
            .build();
        heading.update_property(&[gtk::accessible::Property::Level(1)]);
        let lead = label(&["lead"]);
        let problems = label(&["problems"]);
        problems.set_visible(false);

        let needs = Section::new("Needs you", Some("oldest first"), -1);
        let tries = Section::new("Tries", Some("copies of your folders"), -1);
        let steps = Section::new("Last steps", None, SIDE_CHARS);
        let newer = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(12)
            .css_classes(["newer"])
            .visible(false)
            .build();
        let newer_label = label(&["detail"]);
        newer_label.set_hexpand(true);
        newer_label.set_valign(gtk::Align::Center);
        let show_newer = gtk::Button::with_label("Show them");
        newer.append(&newer_label);
        newer.append(&show_newer);
        steps
            .root
            .insert_child_after(&newer, steps.root.first_child().as_ref());

        let main = gtk::Box::new(gtk::Orientation::Vertical, 24);
        main.set_hexpand(true);
        main.append(&needs.root);
        main.append(&tries.root);
        let side = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["side"])
            .build();
        side.append(&steps.root);
        let columns = gtk::Box::new(gtk::Orientation::Horizontal, 28);
        columns.append(&main);
        columns.append(&side);
        let page = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .css_classes(["page"])
            .build();
        page.append(&heading);
        page.append(&lead);
        page.append(&problems);
        columns.set_margin_top(22);
        page.append(&columns);
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&page)
            .build();
        window.set_child(Some(&scrolled));

        let ui = Rc::new(Ui {
            window,
            options,
            heading,
            lead,
            problems,
            columns,
            side,
            needs,
            tries,
            steps,
            newer,
            newer_label,
            shown_seq: Cell::new(0),
            first: Cell::new(true),
            latest: RefCell::new(Snapshot::default()),
            reader: RefCell::new(None),
            pending: Cell::new(None),
            last_read: Cell::new(None),
            calm: RefCell::new(None),
            sample: RefCell::new(None),
            monitor: RefCell::new(None),
        });

        let weak = Rc::downgrade(&ui);
        show_newer.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.shown_seq.set(0);
                let snap = ui.latest.borrow().clone();
                ui.show_steps(&snap, true);
                ui.steps.list.grab_focus();
            }
        });
        let weak = Rc::downgrade(&ui);
        ui.window.connect_default_width_notify(move |w| {
            if let Some(ui) = weak.upgrade() {
                ui.fit(w.default_width());
            }
        });
        let weak = Rc::downgrade(&ui);
        ui.window.connect_suspended_notify(move |w| {
            if let Some(ui) = weak.upgrade()
                && !w.is_suspended()
                && let Some(kind) = ui.pending.get()
            {
                ui.request(kind);
            }
        });
        ui.fit(w);
        ui
    }

    /// One column on a narrow window, two on a wide one.
    fn fit(&self, width: i32) {
        let narrow = width < NARROW;
        self.columns.set_orientation(if narrow {
            gtk::Orientation::Vertical
        } else {
            gtk::Orientation::Horizontal
        });
    }

    /// Shows the first reading, made while GTK started (or now, if that failed), and then the window, so that its
    /// first frame shows arbor's state.
    fn start(self: &Rc<Ui>, first: Option<(Reader, Snapshot)>) {
        let (reader, snap) = first.unwrap_or_else(|| {
            let mut reader = Reader::new(STEPS);
            let snap = reader.read(Kind::All);
            (reader, snap)
        });
        self.reader.replace(Some(reader));
        self.last_read.set(Some(Instant::now()));
        self.apply(snap);
        self.window.present();
        self.watch();
        // A step recorded between that reading and the watch would otherwise wait for the next one.
        if self.reader.borrow().as_ref().is_some_and(Reader::behind) {
            self.request(Kind::All);
        }
        if let Some(path) = self.options.png.clone() {
            let ui = self.clone();
            glib::timeout_add_local_once(Duration::from_millis(700), move || {
                if let Err(e) = ui.save_png(&path) {
                    eprintln!("arbor-mission: {e}");
                }
                ui.window.close();
            });
        }
        if self.options.bench == Some(Bench::Animate) {
            match self.needs.rows.borrow().first() {
                Some(r) => r.shift.bob(),
                None => eprintln!("arbor-mission: nothing waits for a person, so nothing moves"),
            }
            let weak = Rc::downgrade(self);
            glib::timeout_add_local(Duration::from_millis(100), move || match weak.upgrade() {
                Some(ui) => {
                    ui.request(Kind::All);
                    glib::ControlFlow::Continue
                }
                None => glib::ControlFlow::Break,
            });
        }
    }

    /// Watches the ledger, or the nearest folder above arbor's state folder until arbor makes it.
    fn watch(self: &Rc<Ui>) {
        let Some((path, ready)) = state::watched() else {
            return;
        };
        let file = gio::File::for_path(&path);
        let monitor = if ready {
            file.monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
        } else {
            file.monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
        };
        let Ok(monitor) = monitor else {
            return;
        };
        let weak = Rc::downgrade(self);
        monitor.connect_changed(move |_, _, _, _| {
            if let Some(ui) = weak.upgrade() {
                if !ready && state::watched().is_some_and(|(_, r)| r) {
                    ui.watch();
                }
                ui.request(Kind::All);
            }
        });
        if let Some(old) = self.monitor.replace(Some(monitor)) {
            old.cancel();
        }
    }

    fn calm(&self) -> Duration {
        if self.options.bench == Some(Bench::Animate) {
            Duration::from_millis(100)
        } else {
            CALM
        }
    }

    /// Asks for a reading: at once when the last one is a second old, else once that second has passed. A hidden
    /// window reads nothing until it is shown again.
    fn request(self: &Rc<Ui>, kind: Kind) {
        let kind = match self.pending.get() {
            Some(Kind::All) => Kind::All,
            _ => kind,
        };
        self.pending.set(Some(kind));
        if self.window.is_suspended()
            || self.reader.borrow().is_none()
            || self.calm.borrow().is_some()
        {
            return;
        }
        let wait = self
            .last_read
            .get()
            .map_or(Duration::ZERO, |t| self.calm().saturating_sub(t.elapsed()));
        if wait.is_zero() {
            self.read();
            return;
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(wait, move || {
            if let Some(ui) = weak.upgrade() {
                ui.calm.take();
                ui.read();
            }
        });
        self.calm.replace(Some(id));
    }

    /// Reads arbor's state on a thread of GLib's pool, then shows it.
    fn read(self: &Rc<Ui>) {
        let (Some(kind), Some(mut reader)) = (self.pending.take(), self.reader.take()) else {
            return;
        };
        self.last_read.set(Some(Instant::now()));
        let ui = self.clone();
        glib::spawn_future_local(async move {
            let done = gio::spawn_blocking(move || {
                let snap = reader.read(kind);
                (reader, snap)
            })
            .await;
            let Ok((reader, snap)) = done else {
                eprintln!("arbor-mission: reading arbor's state failed");
                return;
            };
            ui.reader.replace(Some(reader));
            ui.apply(snap);
            if let Some(kind) = ui.pending.get() {
                ui.request(kind);
            }
        });
    }

    /// Shows a reading: only the words that changed are set, so GTK lays out and draws only those lines.
    fn apply(self: &Rc<Ui>, snap: Snapshot) {
        let first = self.first.replace(false);
        let arrive = !first;
        set_text(&self.heading, &words::heading(snap.waiting.len()));
        set_text(&self.lead, &words::lead(&snap));
        let problems = match snap.problems.as_slice() {
            [] => String::new(),
            p => format!(
                "Some of arbor's records could not be read: {}",
                p.join("; ")
            ),
        };
        set_text(&self.problems, &problems);
        self.problems.set_visible(!problems.is_empty());

        let decisions: Vec<Line> = snap
            .waiting
            .iter()
            .map(|e| words::decision(e, snap.now_ns))
            .collect();
        let new = self.needs.show(decisions, arrive);
        if arrive && !new.is_empty() {
            // Only what needs a person is announced, politely, and merged.
            let text = match new.len() {
                1 => format!("A decision is waiting: {}", self.decision_title(&new[0])),
                n => format!("{n} decisions are waiting"),
            };
            self.window
                .announce(&text, gtk::AccessibleAnnouncementPriority::Medium);
        }
        self.tries.show(try_lines(&snap), arrive);
        self.show_steps(&snap, false);
        let rooms_read = snap
            .rooms
            .iter()
            .any(|r| r.mem.is_some() || r.cpu_us.is_some());
        self.latest.replace(snap);
        if rooms_read {
            self.arm_sample();
        }
    }

    fn decision_title(&self, key: &str) -> String {
        self.needs
            .rows
            .borrow()
            .iter()
            .find(|r| r.line.key == key)
            .map(|r| r.line.title.clone())
            .unwrap_or_default()
    }

    /// History holds still: once shown, newer steps wait behind one line until asked for.
    fn show_steps(&self, snap: &Snapshot, asked: bool) {
        let newest = snap.steps.last().map_or(0, |s| s.seq);
        let shown = self.shown_seq.get();
        if asked || shown == 0 || newest < shown {
            let lines = snap
                .steps
                .iter()
                .rev()
                .map(|s| words::step(s, snap.now_ns))
                .collect();
            self.steps.show(lines, false);
            self.side.set_visible(!self.steps.rows.borrow().is_empty());
            self.shown_seq.set(newest.max(1));
            self.newer.set_visible(false);
            if newest == 0 {
                self.shown_seq.set(0);
                set_text(&self.newer_label, "");
            }
        } else if newest > shown {
            let n = newest - shown;
            set_text(
                &self.newer_label,
                &match n {
                    1 => "There is 1 newer step.".to_string(),
                    n => format!("There are {n} newer steps."),
                },
            );
            self.newer.set_visible(true);
        }
    }

    /// Reads what the rooms use again in 10 s.
    fn arm_sample(self: &Rc<Ui>) {
        if self.sample.borrow().is_some() || self.options.bench == Some(Bench::Animate) {
            return;
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_seconds_local_once(SAMPLE_S, move || {
            if let Some(ui) = weak.upgrade() {
                ui.sample.take();
                ui.request(Kind::Rooms);
            }
        });
        self.sample.replace(Some(id));
    }

    fn save_png(&self, path: &std::path::Path) -> Result<(), String> {
        let content = self.window.clone().upcast::<gtk::Widget>();
        let (w, h) = (content.width(), content.height());
        let paintable = gtk::WidgetPaintable::new(Some(&content));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, f64::from(w), f64::from(h));
        let node = snapshot.to_node().ok_or("the window drew nothing")?;
        let renderer = self.window.renderer().ok_or("the window has no renderer")?;
        let bounds = node.bounds();
        let texture = renderer.render_texture(&node, Some(&bounds));
        texture
            .save_to_png(path)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))
    }
}

/// The tries, with what is working first: tries with rooms running, then open ones, then kept ones, each band by
/// name. Each try is followed by its first rooms, oldest first, and one line for the rest; past `TRIES_SHOWN`, the
/// tries fold into one line that counts them.
fn try_lines(snap: &Snapshot) -> Vec<Line> {
    let working = |t: &state::Try| snap.rooms.iter().any(|r| r.world == t.name);
    let band = |t: &state::Try| match (t.kept_ns, working(t)) {
        (None, true) => 0,
        (None, false) => 1,
        (Some(_), _) => 2,
    };
    let mut tries: Vec<&state::Try> = snap.tries.iter().collect();
    tries.sort_by(|a, b| band(a).cmp(&band(b)).then(a.name.cmp(&b.name)));
    let mut lines = Vec::new();
    for t in tries.iter().take(TRIES_SHOWN) {
        let rooms: Vec<&state::Room> = snap.rooms.iter().filter(|r| r.world == t.name).collect();
        lines.push(words::try_line(t, rooms.len(), snap.now_ns));
        for r in rooms.iter().take(ROOMS_SHOWN) {
            lines.push(words::room(r, snap.now_ns));
        }
        if rooms.len() > ROOMS_SHOWN {
            lines.push(words::more_rooms(&t.name, rooms.len() - ROOMS_SHOWN));
        }
    }
    if tries.len() > TRIES_SHOWN {
        let rest = &tries[TRIES_SHOWN..];
        lines.push(words::more_tries(
            rest.len(),
            rest.iter().filter(|t| working(t)).count(),
        ));
    }
    lines
}

fn set_text(l: &gtk::Label, text: &str) {
    if l.label() != text {
        l.set_label(text);
    }
}

/// From GTK 4.20, an app's own style follows the desktop's light or dark setting, contrast and reduced motion only
/// through its provider's media queries, which read these three properties.
fn follow_desktop(provider: &gtk::CssProvider) {
    let Some(settings) = gtk::Settings::default() else {
        return;
    };
    for (setting, query) in [
        ("gtk-interface-color-scheme", "prefers-color-scheme"),
        ("gtk-interface-contrast", "prefers-contrast"),
        ("gtk-interface-reduced-motion", "prefers-reduced-motion"),
    ] {
        if settings.find_property(setting).is_some() && provider.find_property(query).is_some() {
            settings
                .bind_property(setting, provider, query)
                .sync_create()
                .build();
        }
    }
}

/// Sets the window's light or dark look instead of the desktop's (GTK 4.20 and later).
fn set_scheme(dark: bool) {
    let Some(settings) = gtk::Settings::default() else {
        return;
    };
    let Some(class) =
        glib::Type::from_name("GtkInterfaceColorScheme").and_then(glib::EnumClass::with_type)
    else {
        return;
    };
    if let Some(v) = class.value_by_nick(if dark { "dark" } else { "light" }) {
        settings.set_property_from_value("gtk-interface-color-scheme", &v.to_value(&class));
    }
}
