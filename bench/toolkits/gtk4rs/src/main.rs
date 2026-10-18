//! The Mission Control test program of issue #8 in GTK 4 through gtk4-rs, the bindings Mission Control uses: the C
//! program of ../gtk4/main.c, call for call, so that the two measure what the bindings add. The data is made up.
//!   gtk4rs-bench still     rows, then 20 updates of 30 rows from 3 s, the card arriving at 5 s, then nothing
//!   gtk4rs-bench animate   the card moving without end, and 30 rows changing every 100 ms
//! The rows are a GtkListView, which makes widgets for the rows on screen only; the card is moved and faded in its
//! snapshot, from a tick callback of the frame clock, so a frame redraws it without laying the window out again.
use std::cell::{Cell, RefCell};
use std::time::Duration;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, gio, glib, graphene};

const NAMES: [&str; 12] = [
    "Help pages", "Test suite", "Bug reports", "Website", "Invoices", "Payments", "Newsletter", "Photo archive",
    "Sales summary", "Translations", "Release notes", "Search index",
];
const UNITS: [&str; 12] = [
    "pages translated", "tests faster", "reports sorted", "pages lighter", "invoices filed", "files moved",
    "emails ready", "photos sorted", "rows summed", "strings done", "notes written", "files indexed",
];
const CSS: &str = "window { background-color: #fbfaf7; color: #22201c; font-size: 14px; }\
    .heading { font-size: 22px; font-weight: 700; }\
    .card { background-color: #fff3dc; border-radius: 8px; padding: 0 16px; }\
    listview, listview > row { background: none; padding: 0; min-height: 0; }\
    .row { min-height: 32px; padding: 0 16px; }\
    .title { font-weight: 600; }\
    .faint { color: #5c5850; }\
    .mark { border-radius: 5px; min-width: 10px; min-height: 10px; }\
    .mark0 { background-color: #2f7d4f; } .mark1 { background-color: #a86f0c; } .mark2 { background-color: #77736a; }";

// ---- one row: its made-up state, which every toolkit's program computes the same way ----

mod item {
    use super::*;

    #[derive(Default, glib::Properties)]
    #[properties(wrapper_type = super::Item)]
    pub struct Item {
        pub index: Cell<u32>,
        pub done: Cell<u32>,
        pub total: Cell<u32>,
        #[property(get)]
        pub words: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Item {
        const NAME: &'static str = "BenchItem";
        type Type = super::Item;
    }

    #[glib::derived_properties]
    impl ObjectImpl for Item {}
}

glib::wrapper! {
    pub struct Item(ObjectSubclass<item::Item>);
}

impl Item {
    fn new(i: u32) -> Item {
        let item: Item = glib::Object::new();
        let total = 100 + (i * 53) % 900;
        item.imp().index.set(i);
        item.imp().total.set(total);
        item.imp().done.set((i * 37) % total);
        item.set_words();
        item
    }

    fn set_words(&self) {
        let p = self.imp();
        p.words.replace(format!("{} of {} {}", p.done.get(), p.total.get(), UNITS[p.index.get() as usize % 12]));
    }
}

/// Tick k changes the 30 rows whose index ends in k % 10.
fn update(rows: &gio::ListStore, k: u32) {
    let mut i = k % 10;
    while i < 300 {
        let item = rows.item(i).and_downcast::<Item>().unwrap();
        let p = item.imp();
        p.done.set((p.done.get() + 1 + (i + k) % 3).min(p.total.get()));
        item.set_words();
        item.notify_words();
        i += 10;
    }
}

fn every_100ms(rows: gio::ListStore, forever: bool) {
    let ticks = Cell::new(0);
    glib::timeout_add_local(Duration::from_millis(100), move || {
        let k = ticks.get();
        ticks.set(k + 1);
        update(&rows, k);
        if forever || ticks.get() < 20 { glib::ControlFlow::Continue } else { glib::ControlFlow::Break }
    });
}

// ---- the rows of the list ----

fn label(css: Option<&str>, width: i32) -> gtk::Label {
    let l = gtk::Label::new(None);
    l.set_xalign(0.0);
    if width != 0 {
        l.set_size_request(width, -1);
    }
    if let Some(c) = css {
        l.add_css_class(c);
    }
    l
}

fn setup(li: &gtk::ListItem) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("row");
    let mark = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    mark.add_css_class("mark");
    mark.set_valign(gtk::Align::Center);
    row.append(&mark);
    row.append(&label(Some("title"), 200));
    row.append(&label(None, 320));
    row.append(&label(Some("faint"), 120));
    row.append(&label(Some("faint"), 0));
    li.set_child(Some(&row));
}

fn bind(li: &gtk::ListItem) {
    let item = li.item().and_downcast::<Item>().unwrap();
    let mark = li.child().unwrap().first_child().unwrap();
    let title = mark.next_sibling().and_downcast::<gtk::Label>().unwrap();
    let words = title.next_sibling().and_downcast::<gtk::Label>().unwrap();
    let agents = words.next_sibling().and_downcast::<gtk::Label>().unwrap();
    let until = agents.next_sibling().and_downcast::<gtk::Label>().unwrap();
    let i = item.imp().index.get();
    for m in ["mark0", "mark1", "mark2"] {
        mark.remove_css_class(m);
    }
    mark.add_css_class(["mark0", "mark1", "mark2"][i as usize % 3]);
    title.set_text(&format!("{} {}", NAMES[i as usize % 12], i / 12 + 1));
    let binding = item.bind_property("words", &words, "label").sync_create().build();
    unsafe { li.set_data("words", binding) };
    agents.set_text(&format!("{} agents", 1 + (i * 7) % 96));
    until.set_text(&format!("until {}:00", 9 + i % 9));
}

fn unbind(li: &gtk::ListItem) {
    if let Some(b) = unsafe { li.steal_data::<glib::Binding>("words") } {
        b.unbind();
    }
}

// ---- the card: moved and faded as it is drawn ----

mod card {
    use super::*;

    pub struct Card {
        pub y: Cell<f64>,
        pub alpha: Cell<f64>,
        pub start: Cell<i64>,
    }

    impl Default for Card {
        fn default() -> Self {
            Card { y: Cell::new(0.0), alpha: Cell::new(1.0), start: Cell::new(0) }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Card {
        const NAME: &'static str = "BenchCard";
        type Type = super::Card;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk::BinLayout>();
        }
    }

    impl ObjectImpl for Card {
        fn constructed(&self) {
            self.parent_constructed();
            let inside = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            inside.add_css_class("card");
            let text = gtk::Label::new(Some("Payments, moved to version 2: waiting for you. Can be undone."));
            text.set_hexpand(true);
            text.set_xalign(0.0);
            inside.append(&text);
            inside.set_parent(&*self.obj());
        }

        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for Card {
        fn snapshot(&self, s: &gtk::Snapshot) {
            s.save();
            s.translate(&graphene::Point::new(0.0, self.y.get() as f32));
            s.push_opacity(self.alpha.get());
            self.parent_snapshot(s);
            s.pop();
            s.restore();
        }
    }
}

glib::wrapper! {
    pub struct Card(ObjectSubclass<card::Card>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

/// still: the card arrives once, in 200 ms; animate: it moves up and down without end, 200 ms each way.
fn start_moving(card: &Card, animate: bool) {
    card.add_tick_callback(move |w, clock| {
        let c = w.imp();
        let now = clock.frame_time();
        if c.start.get() == 0 {
            c.start.set(now);
        }
        let t = (now - c.start.get()) as f64 / 1e6;
        let mut more = true;
        if animate {
            let phase = (t % 0.4) / 0.2;
            let s = if phase <= 1.0 { phase } else { 2.0 - phase };
            c.y.set(-40.0 * s);
            c.alpha.set(1.0 - 0.75 * s);
        } else {
            let u = (t / 0.2).min(1.0);
            let s = 1.0 - (1.0 - u).powi(3); // ease out
            c.y.set(-40.0 * (1.0 - s));
            c.alpha.set(s);
            more = u < 1.0;
        }
        w.queue_draw();
        if more { glib::ControlFlow::Continue } else { glib::ControlFlow::Break }
    });
}

// ---- the window ----

fn activate(app: &gtk::Application, animate: bool) {
    let css = gtk::CssProvider::new();
    css.load_from_string(CSS);
    gtk::style_context_add_provider_for_display(
        &gdk::Display::default().unwrap(),
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    let window = gtk::ApplicationWindow::new(app);
    window.set_title(Some("arbor bench: GTK 4, gtk4-rs"));
    window.set_default_size(960, 640);
    let bx = gtk::Box::new(gtk::Orientation::Vertical, 8);
    bx.set_margin_start(16);
    bx.set_margin_end(16);
    bx.set_margin_top(16);
    bx.set_margin_bottom(16);
    let heading = gtk::Label::new(Some("3 things need you"));
    heading.add_css_class("heading");
    heading.set_xalign(0.0);
    bx.append(&heading);
    let slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
    slot.set_size_request(-1, 72);
    let card: Card = glib::Object::new();
    card.set_size_request(-1, 64);
    slot.append(&card);
    bx.append(&slot);

    let rows = gio::ListStore::new::<Item>();
    for i in 0..300 {
        rows.append(&Item::new(i));
    }
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, li| setup(li.downcast_ref().unwrap()));
    factory.connect_bind(|_, li| bind(li.downcast_ref().unwrap()));
    factory.connect_unbind(|_, li| unbind(li.downcast_ref().unwrap()));
    let list = gtk::ListView::new(Some(gtk::NoSelection::new(Some(rows.clone()))), Some(factory));
    let scrolled = gtk::ScrolledWindow::new();
    scrolled.set_child(Some(&list));
    scrolled.set_vexpand(true);
    bx.append(&scrolled);
    window.set_child(Some(&bx));

    if animate {
        let c = card.clone();
        glib::timeout_add_local_once(Duration::from_millis(100), move || start_moving(&c, true));
        every_100ms(rows, true);
    } else {
        card.imp().y.set(-40.0);
        card.imp().alpha.set(0.0);
        glib::timeout_add_local_once(Duration::from_millis(3000), move || every_100ms(rows, false));
        let c = card.clone();
        glib::timeout_add_local_once(Duration::from_millis(5000), move || start_moving(&c, false));
    }
    window.present();
}

fn main() -> glib::ExitCode {
    let animate = std::env::args().nth(1).as_deref() == Some("animate");
    let app = gtk::Application::new(Some("arbor.bench"), gio::ApplicationFlags::NON_UNIQUE);
    app.connect_activate(move |app| activate(app, animate));
    // The mode is not a file to open.
    let argv0: Vec<String> = std::env::args().take(1).collect();
    app.run_with_args(&argv0)
}
