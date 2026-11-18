//! `guardian-console`.

#![forbid(unsafe_code)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    println!("guardian-console {}", guardian_console::VERSION);
}
