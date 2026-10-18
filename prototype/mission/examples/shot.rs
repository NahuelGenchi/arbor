//! Draws the window into a PNG file once it shows arbor's state, for the README and pull requests:
//!   cargo run -p mission --example shot -- OUT.png [light|dark] [WIDTHxHEIGHT] [SCENE]
//! It reads arbor's state as the window does, so point ARBOR_HOME (and HOME, for the paths it shows) at made-up
//! worlds: a picture must not show a real computer's folders. SCENE presses buttons first, as a person would:
//! `confirm` (the first decision's Release), `refuse` (its Refuse, with a reason typed), `releasing` (the last
//! decision released, while it runs) or `answered` (the last released and the first refused). Those last two
//! answer for real: `arbor release` runs the decision's command.

use std::process::ExitCode;

use mission::ui::{self, Options, Scene};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(png) = args.first() else {
        eprintln!(
            "usage: shot OUT.png [light|dark] [WIDTHxHEIGHT] [confirm|refuse|releasing|answered]"
        );
        return ExitCode::from(2);
    };
    let dark = match args.get(1).map(String::as_str) {
        Some("dark") => Some(true),
        Some("light") => Some(false),
        _ => None,
    };
    let size = args
        .get(2)
        .and_then(|s| s.split_once('x'))
        .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)));
    let scene = match args.get(3) {
        None => None,
        Some(name) => match Scene::named(name) {
            Some(s) => Some(s),
            None => {
                eprintln!("shot: no scene '{name}'");
                return ExitCode::from(2);
            }
        },
    };
    let code = ui::run(Options {
        png: Some(png.into()),
        scene,
        dark,
        size,
        ..Options::default()
    });
    ExitCode::from(code.get() as u8)
}
