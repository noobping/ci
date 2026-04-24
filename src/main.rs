#![cfg(unix)]

fn main() {
    std::process::exit(ci::entrypoint(std::env::args_os().collect()));
}
