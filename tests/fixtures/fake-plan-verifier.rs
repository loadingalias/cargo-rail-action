use std::env;
use std::ffi::OsStr;
use std::process;

fn main() {
    let arguments = env::args_os().skip(1).collect::<Vec<_>>();
    let expected = ["rail", "plan", "--verify"];
    if arguments.len() != 4
        || arguments
            .iter()
            .take(3)
            .zip(expected)
            .any(|(actual, expected)| actual != OsStr::new(expected))
    {
        eprintln!("unexpected cargo-rail verification arguments: {arguments:?}");
        process::exit(64);
    }

    if let Some(path) = env::var_os("FAKE_CARGO_RAIL_LOG") {
        let mut log = Vec::new();
        for argument in &arguments {
            log.extend_from_slice(argument.to_string_lossy().as_bytes());
            log.push(0);
        }
        if let Err(error) = std::fs::write(path, log) {
            eprintln!("cannot write fake cargo-rail argument log: {error}");
            process::exit(74);
        }
    }
    if let Ok(output) = env::var("FAKE_CARGO_RAIL_STDOUT") {
        print!("{output}");
    }
    if let Ok(output) = env::var("FAKE_CARGO_RAIL_STDERR") {
        eprintln!("{output}");
    }
    let status = env::var("FAKE_CARGO_RAIL_STATUS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    process::exit(status);
}
