//! `arcline` — the desktop app.
//!
//! ```text
//! arcline [--project FILE]                     editor: viewer + transport for a project
//! arcline --embed-parent <HWND> --embed-channel stdio [--poc-seconds N]
//! ```

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
#[allow(dead_code)]
mod editor;
mod embed;
mod poc;
mod shell;

use std::process::ExitCode;

fn parse_args() -> Result<app::Options, String> {
    let mut o = app::Options::default();
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--embed-parent" => {
                let v = it.next().ok_or("--embed-parent needs a window handle")?;
                let h = if let Some(hex) = v.strip_prefix("0x") {
                    isize::from_str_radix(hex, 16)
                } else {
                    v.parse::<isize>()
                };
                o.parent = Some(h.map_err(|_| format!("invalid window handle {v}"))?);
            }
            "--embed-channel" => match it.next().as_deref() {
                Some("stdio") => o.embed_stdio = true,
                other => {
                    return Err(format!(
                        "unsupported embed channel {other:?} (PoC supports: stdio)"
                    ));
                }
            },
            "--poc-seconds" => {
                let v = it.next().ok_or("--poc-seconds needs a value")?;
                o.poc_seconds = Some(v.parse().map_err(|_| format!("invalid seconds {v}"))?);
            }
            "--project" => {
                let v = it.next().ok_or("--project needs a path")?;
                o.project = Some(std::path::PathBuf::from(v));
            }
            "-h" | "--help" => {
                eprintln!(
                    "usage: arcline [--project FILE] [--embed-parent HWND --embed-channel stdio] [--poc-seconds N]"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(o)
}

fn main() -> ExitCode {
    let opts = match parse_args() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("arcline: {e}");
            return ExitCode::from(64);
        }
    };
    let event_loop = match winit::event_loop::EventLoop::<app::UserEvent>::with_user_event().build()
    {
        Ok(el) => el,
        Err(e) => {
            eprintln!("arcline: cannot create event loop: {e}");
            return ExitCode::FAILURE;
        }
    };
    if opts.embed_stdio {
        embed::spawn_stdin_reader(event_loop.create_proxy());
    }
    let mut app = app::App::new(opts);
    match event_loop.run_app(&mut app) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("arcline: {e}");
            ExitCode::FAILURE
        }
    }
}
