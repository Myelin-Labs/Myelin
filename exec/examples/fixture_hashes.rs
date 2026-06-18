use std::env;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = env::args().skip(1).peekable();
    if args.peek().is_none() {
        eprintln!("usage: cargo run -p spora-exec --example fixture_hashes -- <file> [file...]");
        return ExitCode::from(2);
    }

    let mut had_error = false;
    for file in args {
        match fs::read(&file) {
            Ok(bytes) => {
                let hash = blake3::hash(&bytes);
                let name = Path::new(&file).file_name().and_then(|s| s.to_str()).unwrap_or(&file);
                println!("{hash}  {name}");
            }
            Err(err) => {
                had_error = true;
                eprintln!("{file}: {err}");
            }
        }
    }

    if had_error {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
