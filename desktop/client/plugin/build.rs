// The Kotlin side (android/) is built into the app with it; the page calls none of it (the
// Rust shell does, desktop/client/src/android.rs), so the plugin gives the page no commands.
const COMMANDS: &[&str] = &[];

fn main() {
    tauri_plugin::Builder::new(COMMANDS).android_path("android").build();
}
