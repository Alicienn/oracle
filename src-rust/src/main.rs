// Release builds must not open a console window behind the app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod core;
mod ui;

use winit::event_loop::{ControlFlow, EventLoop};

fn main() {
    let event_loop = match EventLoop::new() {
        Ok(event_loop) => event_loop,
        Err(err) => {
            eprintln!("Oracle could not start: {err}");
            std::process::exit(1);
        }
    };

    event_loop.set_control_flow(ControlFlow::Wait);

    if let Err(err) = event_loop.run_app(&mut app::Oracle::default()) {
        eprintln!("Oracle stopped: {err}");
        std::process::exit(1);
    }
}
