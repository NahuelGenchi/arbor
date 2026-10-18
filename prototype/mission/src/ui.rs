//! The window: one column of what needs you and the tries with their rooms, and a side column with the last steps of
//! history, which drops below on a narrow window.
//!
//! How it stays quiet (docs/INTERFACE.md, *How an update reaches the screen*): it watches arbor's ledger with inotify,
//! since every step that changes a world, a cell or the outbox is recorded there, and sleeps until the ledger moves.
//! A change starts a reading at once, and the next one waits until a second has passed, so a busy ledger is read at
//! most once a second. While the ledger keeps moving, the watch rests: the window reads the ledger again on each
//! calm second instead of waking for each of its writes, and watches again once a reading finds nothing new. A
//! reading runs off the main thread and reads only what the new steps name (`state`); the window then changes only
//! the lines whose words changed, and GTK draws them on the next frame. What rooms use changes without a step, so
//! while rooms run and the window is shown, it reads them again every 10 s. A still screen has no timer running.
//!
//! Each decision has Release and Refuse, and each try See changes and Keep, or Undo the keep once kept. A press asks
//! for one confirmation in place, with the focus on Cancel so that a held or repeated Enter never confirms;
//! confirmed, the window runs `arbor release`, `arbor refuse`, `arbor keep` or `arbor undo` off the main thread
//! (`act`), and the result joins the *Done here* list.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{gdk, gio, glib};

use arbor::world;

use crate::act::{self, Ask};
use crate::shift::Shift;
use crate::state::{self, Kind, Reader, Snapshot};
use crate::words::{self, Act, Do, Line, Mark};

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
/// The first column's lines ask for no more than this many characters, so a long one wraps instead of squeezing the
/// side column; a wider window still widens them.
const MAIN_CHARS: i32 = 52;
/// The side column's least width, in px: about SIDE_CHARS characters and its padding.
const SIDE_PX: i32 = 258;
/// More new lines than this at once appear without motion: nobody can follow them all.
const MOVING: usize = 5;
/// How many results the *Done here* list keeps, newest first.
const ANSWERS_SHOWN: usize = 10;

const CSS: &str = include_str!("mission.css");

/// The suite's modes (`--bench`), so that it can measure the window as the test programs of issue #8 are measured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bench {
    /// The window as it is: it shows arbor's state and sleeps.
    Still,
    /// The first line moves without end, and the window reads ten times as often as it would: the ledger every
    /// 100 ms, and the rooms every second.
    Animate,
    /// Answers each decision that this try asked for, releasing and refusing in turn, through the window's own
    /// buttons, and prints how long each took to show, from the press to the frame the compositor showed: the
    /// confirmation, then the result. Then it closes.
    Answer(String),
    /// On this try, three times: shows its changes, keeps it and undoes the keep, through the window's own buttons,
    /// and prints the same times for each. Then it closes.
    Keep(String),
}

/// A state of the window for a picture (`examples/shot.rs`): what a person would see after pressing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scene {
    /// The first decision's Release pressed: its confirmation.
    Confirm,
    /// The first decision's Refuse pressed, with a reason typed.
    Refuse,
    /// The last decision released, while its command runs.
    Releasing,
    /// The last decision released and the first refused, with their receipts.
    Answered,
    /// See changes pressed on the first try that has some, with its first file's lines shown.
    Changes,
    /// Then its Keep pressed: the confirmation.
    Keep,
    /// The first try with changes kept: the result, and Undo the keep on it.
    Kept,
    /// Keep pressed and confirmed on a try whose real folder changed since it began: refused.
    Refused,
}

impl Scene {
    pub fn named(name: &str) -> Option<Scene> {
        Some(match name {
            "confirm" => Scene::Confirm,
            "refuse" => Scene::Refuse,
            "releasing" => Scene::Releasing,
            "answered" => Scene::Answered,
            "changes" => Scene::Changes,
            "keep" => Scene::Keep,
            "kept" => Scene::Kept,
            "refused" => Scene::Refused,
            _ => return None,
        })
    }
}

/// How the window runs.
#[derive(Debug, Clone, Default)]
pub struct Options {
    pub bench: Option<Bench>,
    /// Draw the window into this PNG file once it shows arbor's state, then quit.
    pub png: Option<PathBuf>,
    /// The state to draw it in.
    pub scene: Option<Scene>,
    /// Dark (true) or light (false), instead of the desktop's setting.
    pub dark: Option<bool>,
    pub size: Option<(i32, i32)>,
}

pub fn run(options: Options) -> glib::ExitCode {
    // The first reading of arbor's state runs while GTK starts, so the first frame shows it without waiting for it.
    let first = std::thread::spawn(|| {
        let mut reader = Reader::new(STEPS);
        let snap = reader.read(Kind::LEDGER);
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

/// What a confirmed button asks the window to do: the line's key, what, and the reason typed, if any.
type Go = Rc<dyn Fn(&str, Do, String)>;

/// One list of lines: needs you, answered, tries or steps.
struct Section {
    root: gtk::Box,
    list: gtk::ListBox,
    rows: RefCell<Vec<Row>>,
    /// The side column's lines wrap at this many characters, so that it keeps its width; -1 for no limit.
    max_chars: i32,
    /// What its lines' buttons do once confirmed.
    go: Option<Go>,
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
    output: gtk::Label,
    /// The column the buttons are in, to make them again when they change, and what they do.
    text: gtk::Box,
    go: Option<Go>,
    acts: Option<Rc<Acts>>,
    /// What a try changed, shown under it on request.
    panel: gtk::Box,
}

/// A line's buttons, and the confirmation that one of them opens in place, under the line.
struct Acts {
    key: String,
    defs: Vec<Act>,
    buttons: gtk::Box,
    pressed: Vec<gtk::Button>,
    strip: gtk::Box,
    ask: gtk::Label,
    says: gtk::Label,
    reason: gtk::Entry,
    cancel: gtk::Button,
    go: gtk::Button,
    open: Cell<Option<usize>>,
    on_go: Go,
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
    answers: Section,
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
    /// The effects answered from this window, which Needs you no longer shows, and the answers' lines, newest
    /// first.
    hidden: RefCell<HashSet<u64>>,
    answered: RefCell<Vec<Line>>,
    /// For the suite's `--bench answer` and `--bench keep`: the frame that shows each result, by its line's key.
    result_frame: RefCell<HashMap<String, i64>>,
}

/// Gives a button its own name on the accessibility bus. A button is labelled by its label child, which comes before
/// a name it is given, so that relation goes.
fn named(b: &gtk::Button, name: &str) {
    b.reset_relation(gtk::AccessibleRelation::LabelledBy);
    b.update_property(&[gtk::accessible::Property::Label(name)]);
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
    fn new(title: &str, aside: Option<&str>, max_chars: i32, go: Option<Go>) -> Section {
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
            go,
        }
    }

    /// The view under the line with this key.
    fn panel(&self, key: &str) -> Option<gtk::Box> {
        self.rows
            .borrow()
            .iter()
            .find(|r| r.line.key == key)
            .map(|r| r.panel.clone())
    }

    /// The buttons of the line with this key.
    fn acts(&self, key: &str) -> Option<Rc<Acts>> {
        self.rows
            .borrow()
            .iter()
            .find(|r| r.line.key == key)
            .and_then(|r| r.acts.clone())
    }

    /// Moves the keyboard focus to the line with this key.
    fn focus(&self, key: &str) {
        if let Some(r) = self.rows.borrow().iter().find(|r| r.line.key == key) {
            r.row.grab_focus();
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
                    let r = Row::new(line, self.max_chars, self.go.clone());
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
    fn new(line: Line, max_chars: i32, go: Option<Go>) -> Row {
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
        // What a command printed: as it printed it, and selectable, so it can be copied.
        let output = gtk::Label::builder()
            .xalign(0.0)
            .selectable(true)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::Char)
            .css_classes(["output", "code"])
            .visible(false)
            .build();
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
        text.append(&output);
        let acts = match &go {
            Some(go) if !line.acts.is_empty() => {
                let acts = Acts::new(&line.key, line.acts.clone(), go.clone());
                text.append(&acts.buttons);
                text.append(&acts.strip);
                Some(acts)
            }
            _ => None,
        };
        let panel = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .css_classes(["changes"])
            .visible(false)
            .build();
        text.append(&panel);
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
            output,
            text,
            go,
            acts,
            panel,
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
        if all || old.output != line.output {
            self.output.set_label(line.output.as_deref().unwrap_or(""));
            self.output.set_visible(line.output.is_some());
        }
        if !all && old.acts != line.acts {
            // A try kept or undone has other buttons: they are made again, under the words.
            if let Some(a) = self.acts.take() {
                self.text.remove(&a.buttons);
                self.text.remove(&a.strip);
            }
            if let Some(go) = &self.go
                && !line.acts.is_empty()
            {
                let a = Acts::new(&line.key, line.acts.clone(), go.clone());
                self.text.insert_child_after(&a.buttons, Some(&self.output));
                self.text.insert_child_after(&a.strip, Some(&a.buttons));
                self.acts = Some(a);
            }
            self.panel.set_visible(false);
        }
        if (all || old.mark != line.mark) && line.badge.is_none() {
            for c in ["work", "idle", "kept", "done", "no", "failed", "none"] {
                self.mark.remove_css_class(c);
            }
            self.mark.add_css_class(match line.mark {
                Mark::Work => "work",
                Mark::Idle => "idle",
                Mark::Kept => "kept",
                Mark::Done => "done",
                Mark::No => "no",
                Mark::Failed => "failed",
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

impl Acts {
    fn new(key: &str, defs: Vec<Act>, on_go: Go) -> Rc<Acts> {
        let buttons = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .css_classes(["acts"])
            .build();
        let strip = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .css_classes(["confirm"])
            .visible(false)
            .build();
        let ask = label(&["ask"]);
        let says = label(&["detail"]);
        let reason = gtk::Entry::builder().visible(false).build();
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let cancel = gtk::Button::with_label("Cancel");
        let go = gtk::Button::builder().css_classes(["go"]).build();
        row.append(&cancel);
        row.append(&go);
        strip.append(&ask);
        strip.append(&says);
        strip.append(&reason);
        strip.append(&row);
        // Whoever reaches the buttons by keyboard or screen reader hears what they would do.
        cancel.update_relation(&[gtk::accessible::Relation::DescribedBy(&[
            ask.upcast_ref(),
            says.upcast_ref(),
        ])]);
        go.update_relation(&[gtk::accessible::Relation::DescribedBy(&[says.upcast_ref()])]);
        let pressed = defs
            .iter()
            .map(|a| {
                let b = gtk::Button::builder()
                    .label(&a.label)
                    .css_classes(["act"])
                    .build();
                named(&b, &a.name);
                buttons.append(&b);
                b
            })
            .collect();
        let acts = Rc::new(Acts {
            key: key.to_string(),
            defs,
            buttons,
            pressed,
            strip,
            ask,
            says,
            reason,
            cancel,
            go,
            open: Cell::new(None),
            on_go,
        });
        for (i, b) in acts.pressed.iter().enumerate() {
            let weak = Rc::downgrade(&acts);
            b.connect_clicked(move |_| {
                if let Some(a) = weak.upgrade() {
                    // What changes nothing needs no confirmation.
                    if a.defs[i].ask.is_empty() {
                        (a.on_go)(&a.key, a.defs[i].what, String::new());
                    } else {
                        a.confirm(i);
                    }
                }
            });
        }
        let weak = Rc::downgrade(&acts);
        acts.cancel.connect_clicked(move |_| {
            if let Some(a) = weak.upgrade() {
                a.close();
            }
        });
        let weak = Rc::downgrade(&acts);
        acts.go.connect_clicked(move |_| {
            if let Some(a) = weak.upgrade() {
                a.confirmed();
            }
        });
        let weak = Rc::downgrade(&acts);
        acts.reason.connect_activate(move |_| {
            if let Some(a) = weak.upgrade() {
                a.confirmed();
            }
        });
        // Escape goes back, and the focus returns to the button that opened the confirmation.
        let keys = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(&acts);
        keys.connect_key_pressed(move |_, key, _, _| match weak.upgrade() {
            Some(a) if key == gdk::Key::Escape => {
                a.close();
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        });
        acts.strip.add_controller(keys);
        acts
    }

    /// Opens the confirmation of the `i`th button, with the focus on Cancel.
    fn confirm(&self, i: usize) {
        let Some(a) = self.defs.get(i) else { return };
        self.open.set(Some(i));
        self.ask.set_label(&a.ask);
        self.says.set_label(&a.says);
        self.go.set_label(&a.go);
        for c in ["release", "refuse"] {
            self.go.remove_css_class(c);
        }
        self.go.add_css_class(match a.what {
            Do::Refuse => "refuse",
            _ => "release",
        });
        match &a.reason {
            Some(hint) => {
                self.reason.set_text("");
                self.reason.set_placeholder_text(Some(hint));
                self.reason
                    .update_property(&[gtk::accessible::Property::Label(hint)]);
                self.reason.set_visible(true);
            }
            None => self.reason.set_visible(false),
        }
        self.buttons.set_visible(false);
        self.strip.set_visible(true);
        self.cancel.grab_focus();
    }

    fn close(&self) {
        let Some(i) = self.open.take() else { return };
        self.strip.set_visible(false);
        self.buttons.set_visible(true);
        if let Some(b) = self.pressed.get(i) {
            b.grab_focus();
        }
    }

    fn confirmed(&self) {
        let Some(i) = self.open.take() else { return };
        let why = if WidgetExt::is_visible(&self.reason) {
            self.reason.text().to_string()
        } else {
            String::new()
        };
        self.strip.set_visible(false);
        self.buttons.set_visible(true);
        (self.on_go)(&self.key, self.defs[i].what, why);
    }

    /// The button that does `what`, as a person would press it.
    fn button(&self, what: Do) -> Option<&gtk::Button> {
        let i = self.defs.iter().position(|a| a.what == what)?;
        self.pressed.get(i)
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

        // The buttons reach the window through this, once it is made.
        let me: Rc<RefCell<std::rc::Weak<Ui>>> = Rc::default();
        let go: Go = {
            let me = me.clone();
            Rc::new(move |key, what, why| {
                let ui = me.borrow().upgrade();
                let Some(ui) = ui else { return };
                if let Some(id) = key.strip_prefix("effect ").and_then(|n| n.parse().ok()) {
                    ui.answer(id, what, why);
                } else if let Some(name) = key.strip_prefix("world ") {
                    ui.try_act(name, what);
                }
            })
        };
        let needs = Section::new(
            "Needs you",
            Some("oldest first"),
            MAIN_CHARS,
            Some(go.clone()),
        );
        let answers = Section::new("Done here", Some("newest first"), MAIN_CHARS, None);
        let tries = Section::new(
            "Tries",
            Some("copies of your folders"),
            MAIN_CHARS,
            Some(go),
        );
        let steps = Section::new("Last steps", None, SIDE_CHARS, None);
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
        main.append(&answers.root);
        main.append(&tries.root);
        let side = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["side"])
            .build();
        // Never narrower than its lines' own width: a long line in the first column wraps instead.
        side.set_width_request(SIDE_PX);
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
            answers,
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
            hidden: RefCell::new(HashSet::new()),
            answered: RefCell::new(Vec::new()),
            result_frame: RefCell::new(HashMap::new()),
        });
        me.replace(Rc::downgrade(&ui));

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
            let snap = reader.read(Kind::LEDGER);
            (reader, snap)
        });
        self.reader.replace(Some(reader));
        self.last_read.set(Some(Instant::now()));
        self.apply(snap);
        self.window.present();
        self.watch();
        // A step recorded between that reading and the watch would otherwise wait for the next one.
        if self.reader.borrow().as_ref().is_some_and(Reader::behind) {
            self.request(Kind::LEDGER);
        }
        if let Some(path) = self.options.png.clone() {
            let ui = self.clone();
            glib::spawn_future_local(async move {
                glib::timeout_future(Duration::from_millis(700)).await;
                if let Some(scene) = ui.options.scene {
                    ui.scene(scene).await;
                }
                if let Err(e) = ui.save_png(&path) {
                    eprintln!("arbor-mission: {e}");
                }
                ui.window.close();
            });
        }
        if let Some(Bench::Answer(world)) = self.options.bench.clone() {
            let ui = self.clone();
            glib::spawn_future_local(async move {
                ui.bench_answers(&world).await;
                ui.window.close();
            });
        }
        if let Some(Bench::Keep(world)) = self.options.bench.clone() {
            let ui = self.clone();
            glib::spawn_future_local(async move {
                ui.bench_keeps(&world).await;
                ui.window.close();
            });
        }
        if self.options.bench == Some(Bench::Animate) {
            match self.needs.rows.borrow().first() {
                Some(r) => r.shift.bob(),
                None => eprintln!("arbor-mission: nothing waits for a person, so nothing moves"),
            }
            // Ten times as often as the window reads: the ledger every 100 ms, and the rooms every second.
            let weak = Rc::downgrade(self);
            let mut tick = 0u32;
            glib::timeout_add_local(Duration::from_millis(100), move || match weak.upgrade() {
                Some(ui) => {
                    tick += 1;
                    ui.request(if tick.is_multiple_of(10) {
                        Kind::LEDGER.and(Kind::ROOMS)
                    } else {
                        Kind::LEDGER
                    });
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
                if ready {
                    // The ledger moves: it is followed on the calm second from here, not on each of its writes.
                    ui.rest();
                } else if state::watched().is_some_and(|(_, r)| r) {
                    ui.watch();
                }
                ui.request(Kind::LEDGER);
            }
        });
        if let Some(old) = self.monitor.replace(Some(monitor)) {
            old.cancel();
        }
    }

    /// Stops watching the ledger, so its writes wake nothing.
    fn rest(&self) {
        if let Some(m) = self.monitor.take() {
            m.cancel();
        }
    }

    /// After a reading of the ledger, while the watch rests: one more reading on the calm second if it found new
    /// steps, else the watch again.
    fn follow(self: &Rc<Ui>) {
        if self.monitor.borrow().is_some() || self.pending.get().is_some_and(|k| k.ledger) {
            return;
        }
        let Some(moved) = self.reader.borrow().as_ref().map(Reader::moved) else {
            return;
        };
        if !moved {
            self.watch();
            // A step recorded between that reading and the watch would otherwise wait for the next one.
            if !self.reader.borrow().as_ref().is_some_and(Reader::behind) {
                return;
            }
            self.rest();
        }
        self.request(Kind::LEDGER);
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
        let kind = self.pending.get().map_or(kind, |k| k.and(kind));
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

    /// Reads arbor's state on a thread of GLib's pool, then shows it. A hidden window keeps the ask for later.
    fn read(self: &Rc<Ui>) {
        if self.window.is_suspended() {
            return;
        }
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
            if kind.ledger {
                ui.follow();
            }
            if let Some(kind) = ui.pending.get() {
                ui.request(kind);
            }
        });
    }

    /// Shows a reading: only the words that changed are set, so GTK lays out and draws only those lines.
    fn apply(self: &Rc<Ui>, snap: Snapshot) {
        let first = self.first.replace(false);
        let rooms_read = snap
            .rooms
            .iter()
            .any(|r| r.mem.is_some() || r.cpu_us.is_some());
        self.latest.replace(snap);
        self.show(!first);
        if rooms_read {
            self.arm_sample();
        }
    }

    /// Shows the last reading, with the answers given here: what this window answered no longer needs you.
    fn show(self: &Rc<Ui>, arrive: bool) {
        let snap = self.latest.borrow();
        let hidden = self.hidden.borrow();
        let waiting: Vec<_> = snap
            .waiting
            .iter()
            .filter(|e| !hidden.contains(&e.id))
            .collect();
        set_text(&self.heading, &words::heading(waiting.len()));
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

        let decisions: Vec<Line> = waiting
            .iter()
            .map(|e| words::decision(e, snap.now_ns))
            .collect();
        drop(hidden);
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
        // An answer is the person's own doing: it shows at once, without motion.
        self.answers.show(self.answered.borrow().clone(), false);
        self.tries.show(try_lines(&snap), arrive);
        self.show_steps(&snap, false);
    }

    /// Answers effect `id` as the person confirmed: it leaves Needs you at once, and its line in *Answered* says
    /// that its command runs, then how it ended. A failure puts the decision back.
    fn answer(self: &Rc<Ui>, id: u64, what: Do, why: String) {
        let Some(e) = self
            .latest
            .borrow()
            .waiting
            .iter()
            .find(|e| e.id == id)
            .cloned()
        else {
            return;
        };
        let ask = match what {
            Do::Release => Ask::Release,
            Do::Refuse => Ask::Refuse(why),
            _ => return,
        };
        self.hidden.borrow_mut().insert(id);
        self.put_answer(words::answering(&e, &ask, world::now_ns()));
        let key = format!("answer {id}");
        self.answers.focus(&key);
        let ui = self.clone();
        glib::spawn_future_local(async move {
            let done = gio::spawn_blocking(move || act::answer(id, &ask)).await;
            let line = match done {
                Ok(a) => {
                    if a.failed.is_some() {
                        ui.hidden.borrow_mut().remove(&id);
                    }
                    words::answered(&e, &a, world::now_ns())
                }
                Err(_) => {
                    ui.hidden.borrow_mut().remove(&id);
                    let mut l = words::answering(&e, &Ask::Release, world::now_ns());
                    (l.mark, l.title) =
                        (Mark::Failed, "Not answered: the window lost track".into());
                    l.said = l.title.clone();
                    l
                }
            };
            let news = words::answer_news(&line);
            ui.put_answer(line);
            ui.mark_frame(format!("answer {id}"));
            ui.window
                .announce(&news, gtk::AccessibleAnnouncementPriority::Medium);
            ui.request(Kind::LEDGER);
        });
    }

    /// Notes the frame that will show what just changed, for the suite.
    fn mark_frame(&self, key: String) {
        if let Some(clock) = self.window.frame_clock() {
            self.result_frame
                .borrow_mut()
                .insert(key, clock.frame_counter() + 1);
        }
    }

    /// A try's button: shows or hides what it changed, or keeps it or undoes the keep, as confirmed.
    fn try_act(self: &Rc<Ui>, name: &str, what: Do) {
        let key = format!("world {name}");
        let (Some(panel), Some(t)) = (
            self.tries.panel(&key),
            self.latest
                .borrow()
                .tries
                .iter()
                .find(|t| t.name == name)
                .cloned(),
        ) else {
            return;
        };
        let name = name.to_string();
        let ui = self.clone();
        if what == Do::Changes {
            let button = self
                .tries
                .acts(&key)
                .and_then(|a| a.button(Do::Changes).cloned());
            if panel.is_visible() {
                panel.set_visible(false);
                if let Some(b) = button {
                    b.set_label("See changes");
                }
                return;
            }
            glib::spawn_future_local(async move {
                let look = {
                    let name = name.clone();
                    gio::spawn_blocking(move || act::look(&name)).await
                };
                let Ok(look) = look else { return };
                let view = words::changes(&name, &t.base, &look);
                fill(&panel, &view);
                panel.set_visible(true);
                if let Some(b) = button {
                    b.set_label("Hide changes");
                }
                ui.mark_frame(format!("look {name}"));
                ui.window
                    .announce(&view.news, gtk::AccessibleAnnouncementPriority::Medium);
            });
            return;
        }
        panel.set_visible(false);
        let done_key = format!("try {name}");
        self.put_answer(words::keeping(&name, what, world::now_ns()));
        self.answers.focus(&done_key);
        glib::spawn_future_local(async move {
            let done = {
                let name = name.clone();
                gio::spawn_blocking(move || act::keep_or_undo(&name, what)).await
            };
            let line = match done {
                Ok(d) => words::kept(&d, world::now_ns()),
                Err(_) => {
                    let mut l = words::keeping(&name, what, world::now_ns());
                    (l.mark, l.title) = (Mark::Failed, "The window lost track of it".into());
                    l.said = l.title.clone();
                    l
                }
            };
            let news = words::answer_news(&line);
            ui.put_answer(line);
            ui.mark_frame(done_key);
            ui.window
                .announce(&news, gtk::AccessibleAnnouncementPriority::Medium);
            ui.request(Kind::LEDGER);
        });
    }

    /// Puts an answer's line first in *Done here*, or in place of the one it had, and shows it.
    fn put_answer(self: &Rc<Ui>, line: Line) {
        {
            let mut lines = self.answered.borrow_mut();
            match lines.iter().position(|l| l.key == line.key) {
                Some(i) => lines[i] = line,
                None => {
                    lines.insert(0, line);
                    lines.truncate(ANSWERS_SHOWN);
                }
            }
        }
        self.show(false);
    }

    /// When the compositor showed frame `counter`, in µs of the monotonic clock: GTK's own frame timings, which
    /// it reads from the compositor's presentation feedback. `None` after 2 s without it.
    async fn presented(&self, counter: i64) -> Option<i64> {
        let clock = self.window.frame_clock()?;
        for _ in 0..500 {
            glib::timeout_future(Duration::from_millis(4)).await;
            if let Some(t) = clock.timings(counter)
                && t.is_complete()
            {
                return Some(match t.presentation_time() {
                    0 => t.frame_time(),
                    p => p,
                });
            }
        }
        None
    }

    /// `--bench answer WORLD`: answers the decisions that try WORLD asked for, releasing and refusing in turn,
    /// with the window's own buttons, and prints one JSON line for each: from the press to the frame that shows
    /// the confirmation, and from the confirmation to the frame that shows the result, in ms.
    async fn bench_answers(self: &Rc<Ui>, world: &str) {
        let ids: Vec<u64> = self
            .latest
            .borrow()
            .waiting
            .iter()
            .filter(|e| e.world == world)
            .map(|e| e.id)
            .collect();
        if ids.is_empty() {
            eprintln!("arbor-mission: try {world} asked for nothing");
        }
        for (i, id) in ids.into_iter().enumerate() {
            glib::timeout_future(Duration::from_millis(300)).await;
            let what = if i % 2 == 0 { Do::Release } else { Do::Refuse };
            let (Some(acts), Some(clock)) = (
                self.needs.acts(&format!("effect {id}")),
                self.window.frame_clock(),
            ) else {
                eprintln!("arbor-mission: effect {id} is not shown");
                continue;
            };
            let Some(button) = acts.button(what) else {
                continue;
            };
            let t0 = glib::monotonic_time();
            button.emit_clicked();
            let open = self.presented(clock.frame_counter() + 1).await;
            glib::timeout_future(Duration::from_millis(200)).await;
            let t1 = glib::monotonic_time();
            acts.go.emit_clicked();
            let mut frame = None;
            for _ in 0..15_000 {
                glib::timeout_future(Duration::from_millis(2)).await;
                if let Some(f) = self.result_frame.borrow().get(&format!("answer {id}")) {
                    frame = Some(*f);
                    break;
                }
            }
            let shown = match frame {
                Some(f) => self.presented(f).await,
                None => None,
            };
            let ms = |t: Option<i64>, from: i64| t.map(|t| (t - from) as f64 / 1000.0);
            let failed = self
                .answered
                .borrow()
                .iter()
                .find(|l| l.key == format!("answer {id}"))
                .is_some_and(|l| l.mark == Mark::Failed);
            println!(
                "{}",
                serde_json::json!({
                    "answer": if what == Do::Release { "release" } else { "refuse" },
                    "effect": id,
                    "confirm_ms": ms(open, t0),
                    "result_ms": ms(shown, t1),
                    "failed": failed,
                })
            );
        }
    }

    /// A try's scene: on the first open try with changes whose real folder has not changed, or, for `Refused`, the
    /// first one whose has.
    async fn try_scene(self: &Rc<Ui>, scene: Scene) {
        let open: Vec<String> = self
            .latest
            .borrow()
            .tries
            .iter()
            .filter(|t| t.kept_ns.is_none() && t.changes.unwrap_or(0) > 0)
            .map(|t| t.name.clone())
            .collect();
        let mut chosen = None;
        for name in open {
            let n = name.clone();
            let Ok(look) = gio::spawn_blocking(move || act::look(&n)).await else {
                continue;
            };
            if look.conflicts.is_empty() != (scene == Scene::Refused) {
                chosen = Some(name);
                break;
            }
        }
        let Some(name) = chosen else {
            return eprintln!("arbor-mission: no try fits the scene {scene:?}");
        };
        let press = async |what: Do| {
            if let Some((_, b)) = self.try_button(&name, what).await {
                // Focused first, so the window scrolls to it, as when it is reached by keyboard.
                b.grab_focus();
                b.emit_clicked();
            }
        };
        match scene {
            Scene::Changes | Scene::Keep => {
                press(Do::Changes).await;
                let _ = self.result_shown(&format!("look {name}")).await;
                if let Some(panel) = self.tries.panel(&format!("world {name}")) {
                    open_first_change(&panel);
                }
                if scene == Scene::Keep {
                    press(Do::Keep).await;
                }
                // The view opens under the last try: the picture shows the bottom of the window.
                glib::timeout_future(Duration::from_millis(200)).await;
                if let Some(s) = self.window.child().and_downcast::<gtk::ScrolledWindow>() {
                    let v = s.vadjustment();
                    v.set_value(v.upper() - v.page_size());
                }
            }
            _ => {
                press(Do::Keep).await;
                if let Some(a) = self.tries.acts(&format!("world {name}")) {
                    a.go.emit_clicked();
                }
                let _ = self.result_shown(&format!("try {name}")).await;
                if scene == Scene::Kept {
                    let _ = self.try_button(&name, Do::Undo).await;
                }
            }
        }
        glib::timeout_future(Duration::from_millis(400)).await;
    }

    /// Waits until the frame that shows the result with this key is noted, then for the compositor to show it.
    async fn result_shown(&self, key: &str) -> Option<i64> {
        for _ in 0..15_000 {
            let frame = self.result_frame.borrow_mut().remove(key);
            if let Some(f) = frame {
                return self.presented(f).await;
            }
            glib::timeout_future(Duration::from_millis(2)).await;
        }
        None
    }

    /// Waits until try `name` has a button that does `what`, as the reading after a keep or an undo gives it.
    async fn try_button(&self, name: &str, what: Do) -> Option<(Rc<Acts>, gtk::Button)> {
        for _ in 0..2500 {
            if let Some(a) = self.tries.acts(&format!("world {name}"))
                && let Some(b) = a.button(what).cloned()
            {
                return Some((a, b));
            }
            glib::timeout_future(Duration::from_millis(2)).await;
        }
        None
    }

    /// `--bench keep WORLD`: three times, shows what try WORLD changed, keeps it and undoes the keep, with the
    /// window's own buttons, and prints one JSON line for each: from the press to the frame that shows the view or
    /// the confirmation, and from the confirmation to the frame that shows the result, in ms.
    async fn bench_keeps(self: &Rc<Ui>, world: &str) {
        let ms = |t: Option<i64>, from: i64| t.map(|t| (t - from) as f64 / 1000.0);
        let failed = |ui: &Ui| {
            ui.answered
                .borrow()
                .iter()
                .find(|l| l.key == format!("try {world}"))
                .is_some_and(|l| l.mark == Mark::Failed)
        };
        for _ in 0..3 {
            for what in [Do::Changes, Do::Keep, Do::Undo] {
                glib::timeout_future(Duration::from_millis(300)).await;
                let (Some((acts, button)), Some(clock)) = (
                    self.try_button(world, what).await,
                    self.window.frame_clock(),
                ) else {
                    eprintln!("arbor-mission: try {world} has no button for {what:?}");
                    return;
                };
                let t0 = glib::monotonic_time();
                button.emit_clicked();
                let (open, shown) = if what == Do::Changes {
                    (self.result_shown(&format!("look {world}")).await, None)
                } else {
                    let open = self.presented(clock.frame_counter() + 1).await;
                    glib::timeout_future(Duration::from_millis(200)).await;
                    let t1 = glib::monotonic_time();
                    acts.go.emit_clicked();
                    let shown = self.result_shown(&format!("try {world}")).await;
                    (open, ms(shown, t1))
                };
                println!(
                    "{}",
                    serde_json::json!({
                        "act": format!("{what:?}").to_lowercase(),
                        "open_ms": ms(open, t0),
                        "result_ms": shown,
                        "failed": what != Do::Changes && failed(self),
                    })
                );
            }
        }
    }

    /// Puts the window in a state for a picture.
    async fn scene(self: &Rc<Ui>, scene: Scene) {
        if matches!(
            scene,
            Scene::Changes | Scene::Keep | Scene::Kept | Scene::Refused
        ) {
            return self.try_scene(scene).await;
        }
        let ids: Vec<u64> = self.latest.borrow().waiting.iter().map(|e| e.id).collect();
        let (Some(&first), Some(&last)) = (ids.first(), ids.last()) else {
            return eprintln!("arbor-mission: nothing waits, so there is nothing to answer");
        };
        let acts = |id: u64| self.needs.acts(&format!("effect {id}"));
        let press = |id: u64, what: Do| {
            if let Some(a) = acts(id)
                && let Some(b) = a.button(what)
            {
                b.emit_clicked();
            }
        };
        let done = |id: u64| {
            self.answered
                .borrow()
                .iter()
                .any(|l| l.key == format!("answer {id}") && l.mark != Mark::Work)
        };
        match scene {
            Scene::Confirm => press(first, Do::Release),
            Scene::Refuse => {
                press(first, Do::Refuse);
                if let Some(a) = acts(first) {
                    a.reason.set_text("not yet: the tests come first");
                    a.reason.grab_focus();
                }
            }
            Scene::Changes | Scene::Keep | Scene::Kept | Scene::Refused => {}
            Scene::Releasing | Scene::Answered => {
                press(last, Do::Release);
                if let Some(a) = acts(last) {
                    a.go.emit_clicked();
                }
                if scene == Scene::Answered {
                    if first != last {
                        press(first, Do::Refuse);
                        if let Some(a) = acts(first) {
                            a.reason.set_text("not yet: the tests come first");
                            a.go.emit_clicked();
                        }
                    }
                    for _ in 0..1500 {
                        if done(first) && done(last) {
                            break;
                        }
                        glib::timeout_future(Duration::from_millis(20)).await;
                    }
                }
            }
        }
        glib::timeout_future(Duration::from_millis(400)).await;
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
                ui.request(Kind::ROOMS);
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

/// Fills the view of what a try changed: a note when a keep would be refused, then each folder's files, each with
/// a button that shows its lines.
fn fill(panel: &gtk::Box, view: &words::Changes) {
    while let Some(child) = panel.first_child() {
        panel.remove(&child);
    }
    if let Some(note) = &view.note {
        let l = label(&["warn"]);
        l.set_label(note);
        panel.append(&l);
    }
    for (dir, files) in &view.groups {
        let head = label(&["folder"]);
        head.set_label(dir);
        panel.append(&head);
        for f in files {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            let word = label(&["word"]);
            word.set_label(&f.word);
            word.set_hexpand(false);
            let name = label(&["code"]);
            name.set_label(&f.name);
            name.set_hexpand(true);
            line.append(&word);
            line.append(&name);
            let file = gtk::Box::new(gtk::Orientation::Vertical, 4);
            file.append(&line);
            if let Some(patch) = &f.patch {
                let lines = gtk::Label::builder()
                    .xalign(0.0)
                    .selectable(true)
                    .wrap(true)
                    .wrap_mode(gtk::pango::WrapMode::Char)
                    .css_classes(["output", "code", "patch"])
                    .label(patch)
                    .visible(false)
                    .build();
                let show = gtk::Button::builder()
                    .label("Show the change")
                    .css_classes(["act", "small"])
                    .build();
                show.update_property(&[gtk::accessible::Property::Label(&f.show)]);
                let shown = lines.clone();
                show.connect_clicked(move |b| {
                    let open = !shown.is_visible();
                    shown.set_visible(open);
                    b.set_label(if open {
                        "Hide the change"
                    } else {
                        "Show the change"
                    });
                });
                line.append(&show);
                file.append(&lines);
            }
            panel.append(&file);
        }
    }
}

/// Presses the first "Show the change" in a view of changes, for a picture.
fn open_first_change(panel: &gtk::Box) {
    let mut todo = vec![panel.clone().upcast::<gtk::Widget>()];
    while let Some(w) = todo.pop() {
        if let Some(b) = w.downcast_ref::<gtk::Button>() {
            b.grab_focus();
            b.emit_clicked();
            return;
        }
        let mut child = w.last_child();
        while let Some(c) = child {
            child = c.prev_sibling();
            todo.push(c);
        }
    }
}

/// The tries, with what is working first: tries with rooms running, then open ones, then kept ones, each band by
/// name. Each try is followed by its first rooms, oldest first, and one line for the rest; past `TRIES_SHOWN`, the
/// tries fold into one line that counts them.
fn try_lines(snap: &Snapshot) -> Vec<Line> {
    // Each try's rooms, gathered once: hundreds of tries and rooms would otherwise be compared pair by pair.
    let mut rooms_in: HashMap<&str, Vec<&state::Room>> = HashMap::new();
    for r in &snap.rooms {
        rooms_in.entry(r.world.as_str()).or_default().push(r);
    }
    let working = |t: &state::Try| rooms_in.contains_key(t.name.as_str());
    let band = |t: &state::Try| match (t.kept_ns, working(t)) {
        (None, true) => 0,
        (None, false) => 1,
        (Some(_), _) => 2,
    };
    let mut tries: Vec<&state::Try> = snap.tries.iter().collect();
    tries.sort_by(|a, b| band(a).cmp(&band(b)).then(a.name.cmp(&b.name)));
    let mut lines = Vec::new();
    for t in tries.iter().take(TRIES_SHOWN) {
        let rooms = rooms_in.get(t.name.as_str()).map_or(&[][..], Vec::as_slice);
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
