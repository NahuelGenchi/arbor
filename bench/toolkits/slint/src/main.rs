//! The Mission Control test program of issue #8 in Slint: a window with 300 rows that update, a short animation, then
//! idle. The same program exists for every candidate toolkit (see ../README.md); the data is made up.
//!   slint-bench still     rows, then 20 updates of 30 rows from 3 s, the card arriving at 5 s, then nothing
//!   slint-bench animate   the card moving without end, and 30 rows changing every 100 ms
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use slint::{Model, ModelRc, SharedString, Timer, TimerMode, VecModel};

slint::slint! {
    import { ListView } from "std-widgets.slint";

    export struct Row { mark: int, title: string, words: string, agents: string, until: string }

    component RowView inherits Rectangle {
        in property <Row> item;
        height: 32px;
        HorizontalLayout {
            padding-left: 16px;
            padding-right: 16px;
            spacing: 12px;
            VerticalLayout {
                alignment: center;
                Rectangle {
                    width: 10px;
                    height: 10px;
                    border-radius: 5px;
                    background: item.mark == 1 ? #a86f0c : item.mark == 2 ? #77736a : #2f7d4f;
                }
            }
            Text { text: item.title; width: 200px; vertical-alignment: center; font-weight: 600; color: #22201c; }
            Text { text: item.words; width: 320px; vertical-alignment: center; color: #22201c; }
            Text { text: item.agents; width: 120px; vertical-alignment: center; color: #5c5850; }
            Text { text: item.until; vertical-alignment: center; color: #5c5850; }
        }
    }

    export component AppWindow inherits Window {
        title: "arbor bench: Slint";
        default-font-size: 14px;
        preferred-width: 960px;
        preferred-height: 640px;
        background: #fbfaf7;
        in property <[Row]> rows;
        // still: the card arrives once, in 200 ms
        in property <bool> shown;
        // animate: the card moves up and down without end, 200 ms each way
        in property <bool> looping;

        VerticalLayout {
            padding: 16px;
            spacing: 8px;
            Text { text: "3 things need you"; font-size: 22px; font-weight: 700; color: #22201c; }
            Rectangle {
                height: 72px;
                Rectangle {
                    width: parent.width;
                    height: 64px;
                    y: root.shown ? 0px : -40px;
                    opacity: root.shown ? 1 : 0;
                    animate y, opacity { duration: 200ms; easing: ease-out; }
                    Rectangle {
                        width: parent.width;
                        height: parent.height;
                        y: root.looping ? -40px : 0px;
                        opacity: root.looping ? 0.25 : 1;
                        animate y, opacity { duration: 200ms; easing: linear; iteration-count: -1; direction: alternate; }
                        background: #fff3dc;
                        border-radius: 8px;
                        Text {
                            x: 16px;
                            height: parent.height;
                            vertical-alignment: center;
                            text: "Payments, moved to version 2: waiting for you. Can be undone.";
                            color: #22201c;
                        }
                    }
                }
            }
            ListView {
                for r in root.rows: RowView { item: r; }
            }
        }
    }
}

const NAMES: [&str; 12] = [
    "Help pages", "Test suite", "Bug reports", "Website", "Invoices", "Payments", "Newsletter", "Photo archive",
    "Sales summary", "Translations", "Release notes", "Search index",
];
const UNITS: [&str; 12] = [
    "pages translated", "tests faster", "reports sorted", "pages lighter", "invoices filed", "files moved",
    "emails ready", "photos sorted", "rows summed", "strings done", "notes written", "files indexed",
];

/// The made-up state of one row: every toolkit's program computes the same numbers.
struct Data {
    done: u32,
    total: u32,
}

fn data(i: usize) -> Data {
    let total = 100 + (i as u32 * 53) % 900;
    Data { done: (i as u32 * 37) % total, total }
}

fn row(i: usize, d: &Data) -> Row {
    Row {
        mark: (i % 3) as i32,
        title: SharedString::from(format!("{} {}", NAMES[i % 12], i / 12 + 1)),
        words: SharedString::from(format!("{} of {} {}", d.done, d.total, UNITS[i % 12])),
        agents: SharedString::from(format!("{} agents", 1 + (i * 7) % 96)),
        until: SharedString::from(format!("until {}:00", 9 + i % 9)),
    }
}

/// Tick k changes the 30 rows whose index ends in k % 10.
fn update(model: &VecModel<Row>, state: &mut [Data], k: usize) {
    for i in (k % 10..state.len()).step_by(10) {
        let d = &mut state[i];
        d.done = (d.done + 1 + ((i + k) % 3) as u32).min(d.total);
        model.set_row_data(i, row(i, d));
    }
}

fn main() -> Result<(), slint::PlatformError> {
    let animate = std::env::args().nth(1).as_deref() == Some("animate");
    let app = AppWindow::new()?;
    slint::set_xdg_app_id("arbor-bench")?;
    let state: Vec<Data> = (0..300).map(data).collect();
    let model = Rc::new(VecModel::from(state.iter().enumerate().map(|(i, d)| row(i, d)).collect::<Vec<_>>()));
    app.set_rows(ModelRc::from(model.clone()));
    let state = Rc::new(std::cell::RefCell::new(state));
    let ticks = Rc::new(Cell::new(0usize));
    let updates = Rc::new(Timer::default());
    let tick = {
        let (model, state, ticks, updates) = (model.clone(), state.clone(), ticks.clone(), updates.clone());
        move || {
            let k = ticks.get();
            update(&model, &mut state.borrow_mut(), k);
            ticks.set(k + 1);
            if !animate && k + 1 == 20 {
                updates.stop();
            }
        }
    };
    let starts = Timer::default();
    let arrives = Timer::default();
    if animate {
        app.set_shown(true);
        let weak = app.as_weak();
        starts.start(TimerMode::SingleShot, Duration::from_millis(100), move || {
            if let Some(app) = weak.upgrade() {
                app.set_looping(true);
            }
        });
        updates.start(TimerMode::Repeated, Duration::from_millis(100), tick);
    } else {
        let updates2 = updates.clone();
        starts.start(TimerMode::SingleShot, Duration::from_secs(3), move || {
            updates2.start(TimerMode::Repeated, Duration::from_millis(100), tick.clone());
        });
        let weak = app.as_weak();
        arrives.start(TimerMode::SingleShot, Duration::from_secs(5), move || {
            if let Some(app) = weak.upgrade() {
                app.set_shown(true);
            }
        });
    }
    app.run()
}
