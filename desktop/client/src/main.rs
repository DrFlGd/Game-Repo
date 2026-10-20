// Game Repo Go on Windows (and the Linux build tried in the session); the app itself is the
// library, src/lib.rs, which Android loads as it is.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    gamerepo_client_lib::run()
}
