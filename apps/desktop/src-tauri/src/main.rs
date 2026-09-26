#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // `aeria merge-driver ...` runs for Git without opening a window.
    if let Some(code) = aeria_desktop_lib::run_command_line() {
        std::process::exit(code);
    }
    aeria_desktop_lib::run();
}
