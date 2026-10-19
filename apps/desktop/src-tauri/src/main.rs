#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if desktop_lib::run_contributor_if_requested() {
        return;
    }
    desktop_lib::run()
}
