//! A box that can move its child up and fade it as it is drawn, for the brief's motion: a line arriving moves and
//! fades in 200 ms, once. Only the drawing changes, never the layout, so a frame of motion redraws the child and lays
//! nothing out again. With the desktop's animations turned off (reduced motion), a line just appears.

use std::cell::{Cell, RefCell};

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{glib, graphene};

/// How a line arrives: from this far above, fully transparent, to its place in `ARRIVE_US`, easing out.
const ARRIVE_PX: f32 = 12.0;
const ARRIVE_US: f64 = 200_000.0;

mod imp {
    use super::*;

    pub struct Shift {
        pub y: Cell<f32>,
        pub alpha: Cell<f32>,
        pub tick: RefCell<Option<gtk::TickCallbackId>>,
    }

    impl Default for Shift {
        fn default() -> Self {
            Shift {
                y: Cell::new(0.0),
                alpha: Cell::new(1.0),
                tick: RefCell::new(None),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Shift {
        const NAME: &'static str = "MissionShift";
        type Type = super::Shift;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk::BinLayout>();
        }
    }

    impl ObjectImpl for Shift {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for Shift {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let (y, alpha) = (self.y.get(), self.alpha.get());
            if y == 0.0 && alpha >= 1.0 {
                return self.parent_snapshot(snapshot);
            }
            snapshot.save();
            snapshot.translate(&graphene::Point::new(0.0, y));
            snapshot.push_opacity(f64::from(alpha));
            self.parent_snapshot(snapshot);
            snapshot.pop();
            snapshot.restore();
        }
    }
}

glib::wrapper! {
    pub struct Shift(ObjectSubclass<imp::Shift>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Shift {
    pub fn new(child: &impl IsA<gtk::Widget>) -> Shift {
        let shift: Shift = glib::Object::new();
        child.set_parent(&shift);
        shift
    }

    fn place(&self, y: f32, alpha: f32) {
        self.imp().y.set(y);
        self.imp().alpha.set(alpha);
        self.queue_draw();
    }

    /// Runs `step` on every frame, from the frame clock, until it returns false; a newer motion takes over.
    fn moving(&self, step: impl Fn(&Shift, f64) -> bool + 'static) {
        if let Some(old) = self.imp().tick.take() {
            old.remove();
        }
        let start = Cell::new(None);
        let id = self.add_tick_callback(move |shift, clock| {
            let now = clock.frame_time();
            let t0 = start.get().unwrap_or(now);
            start.set(Some(t0));
            let t = (now - t0) as f64;
            if step(shift, t) {
                glib::ControlFlow::Continue
            } else {
                shift.imp().tick.take();
                glib::ControlFlow::Break
            }
        });
        self.imp().tick.replace(Some(id));
    }

    /// A line arriving: it moves down into its place as it fades in, once. Nothing moves under reduced motion.
    pub fn arrive(&self) {
        if !animations() {
            return;
        }
        self.place(-ARRIVE_PX, 0.0);
        self.moving(|shift, t| {
            let u = (t / ARRIVE_US).min(1.0);
            let s = (1.0 - (1.0 - u).powi(3)) as f32; // ease out
            shift.place(-ARRIVE_PX * (1.0 - s), s);
            u < 1.0
        });
    }

    /// For the suite only (`--bench animate`): the motion of the test program of issue #8, without end. The line rises
    /// 40 px while it fades to 25%, and comes back, 200 ms each way, so that every refresh has a frame to time.
    pub fn bob(&self) {
        self.moving(|shift, t| {
            let phase = (t % 400_000.0) / 200_000.0;
            let s = if phase <= 1.0 { phase } else { 2.0 - phase } as f32;
            shift.place(-40.0 * s, 1.0 - 0.75 * s);
            true
        });
    }
}

/// Whether the desktop allows motion: GNOME's "reduce animation" turns animations off, and from GTK 4.22 a desktop
/// may also ask for reduced motion.
fn animations() -> bool {
    let Some(settings) = gtk::Settings::default() else {
        return true;
    };
    let reduced = settings
        .find_property("gtk-interface-reduced-motion")
        .and_then(|_| {
            let v = settings.property_value("gtk-interface-reduced-motion");
            glib::EnumValue::from_value(&v).map(|(_, e)| e.nick() == "reduce")
        })
        .unwrap_or(false);
    settings.is_gtk_enable_animations() && !reduced
}
