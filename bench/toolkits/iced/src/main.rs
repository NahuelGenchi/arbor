//! The Mission Control test program of issue #8 in iced: a window with 300 rows that update, a short animation, then
//! idle. The same program exists for every candidate toolkit (see ../README.md); the data is made up.
//!   iced-bench still     rows, then 20 updates of 30 rows from 3 s, the card arriving at 5 s, then nothing
//!   iced-bench animate   the card moving without end, and 30 rows changing every 100 ms
use std::time::{Duration, Instant};

use iced::animation::Easing;
use iced::widget::{column, container, pin, row, scrollable, space, text};
use iced::{Animation, Background, Border, Color, Element, Font, Length, Settings, Size, Subscription, Task, Theme, font,
           time, window};

const NAMES: [&str; 12] = [
    "Help pages", "Test suite", "Bug reports", "Website", "Invoices", "Payments", "Newsletter", "Photo archive",
    "Sales summary", "Translations", "Release notes", "Search index",
];
const UNITS: [&str; 12] = [
    "pages translated", "tests faster", "reports sorted", "pages lighter", "invoices filed", "files moved",
    "emails ready", "photos sorted", "rows summed", "strings done", "notes written", "files indexed",
];
const INK: Color = Color::from_rgb8(0x22, 0x20, 0x1c);
const FAINT: Color = Color::from_rgb8(0x5c, 0x58, 0x50);

/// The made-up state of one row: every toolkit's program computes the same numbers.
struct Data {
    done: u32,
    total: u32,
}

fn data(i: usize) -> Data {
    let total = 100 + (i as u32 * 53) % 900;
    Data { done: (i as u32 * 37) % total, total }
}

#[derive(Debug, Clone)]
enum Message {
    Begin,
    Tick,
    Frame(Instant),
}

struct Bench {
    animate: bool,
    rows: Vec<Data>,
    ticks: usize,
    begun: bool,
    // still: the card arrives once, in 200 ms; animate: it moves up and down without end, 200 ms each way
    card: Animation<bool>,
    now: Instant,
}

impl Bench {
    fn new(animate: bool) -> Self {
        let now = Instant::now();
        let card = if animate {
            Animation::new(false).duration(Duration::from_millis(200)).easing(Easing::Linear).repeat_forever()
                .auto_reverse().go(true, now)
        } else {
            Animation::new(false).duration(Duration::from_millis(200)).easing(Easing::EaseOut)
        };
        Bench { animate, rows: (0..300).map(data).collect(), ticks: 0, begun: animate, card, now }
    }

    /// Tick k changes the 30 rows whose index ends in k % 10.
    fn tick(&mut self) {
        let k = self.ticks;
        for i in (k % 10..self.rows.len()).step_by(10) {
            let d = &mut self.rows[i];
            d.done = (d.done + 1 + ((i + k) % 3) as u32).min(d.total);
        }
        self.ticks += 1;
        if !self.animate && self.ticks == 20 {
            self.now = Instant::now();
            self.card.go_mut(true, self.now);
        }
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::Begin => self.begun = true,
            Message::Tick => self.tick(),
            Message::Frame(at) => self.now = at,
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subs = Vec::new();
        if !self.begun {
            subs.push(time::every(Duration::from_secs(3)).map(|_| Message::Begin));
        } else if self.animate || self.ticks < 20 {
            subs.push(time::every(Duration::from_millis(100)).map(|_| Message::Tick));
        }
        if self.card.is_animating(self.now) {
            subs.push(window::frames().map(Message::Frame));
        }
        Subscription::batch(subs)
    }

    fn view(&self) -> Element<'_, Message> {
        let s: f32 = self.card.interpolate(0.0, 1.0, self.now);
        let (y, alpha) = if self.animate { (-40.0 * s, 1.0 - 0.75 * s) } else { (-40.0 * (1.0 - s), s) };
        let card = container(text("Payments, moved to version 2: waiting for you. Can be undone.")
            .color(Color { a: alpha, ..INK }))
            .padding([0, 16])
            .center_y(64)
            .width(Length::Fill)
            .style(move |_: &Theme| container::Style {
                background: Some(Background::Color(Color { a: alpha, ..Color::from_rgb8(0xff, 0xf3, 0xdc) })),
                border: Border { radius: 8.0.into(), ..Border::default() },
                ..container::Style::default()
            });
        let bold = Font { weight: font::Weight::Semibold, ..Font::DEFAULT };
        let rows = column(self.rows.iter().enumerate().map(|(i, d)| {
            let mark = match i % 3 {
                1 => Color::from_rgb8(0xa8, 0x6f, 0x0c),
                2 => Color::from_rgb8(0x77, 0x73, 0x6a),
                _ => Color::from_rgb8(0x2f, 0x7d, 0x4f),
            };
            row![
                container(space()).width(10).height(10).style(move |_: &Theme| container::Style {
                    background: Some(Background::Color(mark)),
                    border: Border { radius: 5.0.into(), ..Border::default() },
                    ..container::Style::default()
                }),
                text(format!("{} {}", NAMES[i % 12], i / 12 + 1)).font(bold).color(INK).width(200),
                text(format!("{} of {} {}", d.done, d.total, UNITS[i % 12])).color(INK).width(320),
                text(format!("{} agents", 1 + (i * 7) % 96)).color(FAINT).width(120),
                text(format!("until {}:00", 9 + i % 9)).color(FAINT),
            ]
            .spacing(12)
            .padding([0, 16])
            .height(32)
            .align_y(iced::Center)
            .into()
        }));
        container(column![
            text("3 things need you").size(22).font(Font { weight: font::Weight::Bold, ..Font::DEFAULT }).color(INK),
            container(pin(card).y(y)).height(72),
            scrollable(rows).height(Length::Fill),
        ]
        .spacing(8))
        .padding(16)
        .style(|_: &Theme| container::Style {
            background: Some(Background::Color(Color::from_rgb8(0xfb, 0xfa, 0xf7))),
            ..container::Style::default()
        })
        .into()
    }
}

fn main() -> iced::Result {
    let animate = std::env::args().nth(1).as_deref() == Some("animate");
    iced::application(move || (Bench::new(animate), Task::none()), Bench::update, Bench::view)
        .title("arbor bench: iced")
        .subscription(Bench::subscription)
        .settings(Settings { default_text_size: 14.into(), ..Settings::default() })
        .window(window::Settings {
            size: Size::new(960.0, 640.0),
            platform_specific: window::settings::PlatformSpecific {
                application_id: "arbor-bench".into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .run()
}
