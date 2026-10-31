// Game Repo Go on Windows and Linux; the app itself is the library, src/lib.rs, which
// Android loads as it is.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // its saves synced in the background (D186), by Task Scheduler or a systemd timer: no window
    #[cfg(desktop)]
    {
        let args: Vec<String> = std::env::args().collect();
        if let Some(i) = args.iter().position(|a| a == "--sync-saves") {
            let data = std::path::PathBuf::from(args.get(i + 1).cloned().unwrap_or_default());
            std::process::exit(if gamerepo_client_lib::sync_saves_alone(&data) { 0 } else { 1 });
        }
    }
    #[cfg(target_os = "linux")]
    {
        let args: Vec<String> = std::env::args().collect();
        // its Steam helper (D109): started apart from it and from Steam by Restart Steam
        if let Some(i) = args.iter().position(|a| a == "--steam-apply") {
            let data = std::path::PathBuf::from(args.get(i + 1).cloned().unwrap_or_default());
            let ok = gamerepo_device::steam::helper(&data, args.iter().any(|a| a == "--game-mode")).is_ok();
            std::process::exit(if ok { 0 } else { 1 });
        }
        // under Game Mode's compositor WebKitGTK's DMA-BUF renderer can leave the window
        // blank (D107)
        if gamerepo_device::steam::game_mode() && std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
    }
    gamerepo_client_lib::run()
}
