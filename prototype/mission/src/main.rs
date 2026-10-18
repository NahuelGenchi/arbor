//! `arbor-mission`: the Mission Control window.

use std::process::ExitCode;

use mission::ui::{self, Bench, Options};

const USAGE: &str = "usage: arbor-mission

Shows what needs you, the tries with the rooms working in them, and the last steps of history, from arbor's state
folder ($ARBOR_HOME, else ~/.local/share/arbor). Release and Refuse on a decision, and Keep and Undo the keep
on a try, run `arbor release`, `arbor refuse`, `arbor keep` and `arbor undo`, the arbor next to this program or
else the one on the PATH, after you confirm. Start an agent makes a try of a folder and a key for it alone
(`arbor new`, `arbor key new`), then opens a terminal ($ARBOR_TERMINAL, else the desktop's) that runs the agent
there (`arbor run`).";

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
        ["--bench", "keep", world] => Some(Bench::Keep(world.to_string())),
        ["--bench", "start", folder] => Some(Bench::Start(folder.into())),
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
