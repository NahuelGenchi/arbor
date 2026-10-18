//! `arbor-mission`: the Mission Control window.

use std::process::ExitCode;

use mission::ui::{self, Bench, Options};

const USAGE: &str = "usage: arbor-mission

Shows what needs you, the tries with the rooms working in them, and the last steps of history, from arbor's state
folder ($ARBOR_HOME, else ~/.local/share/arbor). Release and Refuse on a decision run `arbor release` and
`arbor refuse`, the arbor next to this program or else the one on the PATH, after you confirm.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let bench = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] => None,
        // The suite's modes (bench/sections/mission.py): the window as it is, or with a line that moves without end.
        ["--bench", "still"] => Some(Bench::Still),
        ["--bench", "animate"] => Some(Bench::Animate),
        ["--bench", "answer", world] => Some(Bench::Answer(world.to_string())),
        ["-h" | "--help"] => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let code = ui::run(Options {
        bench,
        ..Options::default()
    });
    ExitCode::from(code.get() as u8)
}
