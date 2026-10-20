//! Game Repo Go's Android side (docs/PLAN.md, "Phase 9b design", D97), its Kotlin in
//! `android/` (`DevicePlugin`): what only Android can do, asked for by the Rust shell
//! (desktop/client/src/android.rs) and never by the page. Elsewhere it registers nothing.

use tauri::plugin::{Builder, TauriPlugin};
use tauri::Runtime;

/// The Kotlin plugin, kept in the app's state on Android.
#[cfg(target_os = "android")]
pub struct Android<R: Runtime>(tauri::plugin::PluginHandle<R>);

#[cfg(target_os = "android")]
impl<R: Runtime> Clone for Android<R> {
    fn clone(&self) -> Self {
        Android(self.0.clone())
    }
}

#[cfg(target_os = "android")]
impl<R: Runtime> Android<R> {
    /// One of `DevicePlugin`'s commands. Never from Android's main thread: the answer comes
    /// from there.
    pub fn call<T: serde::de::DeserializeOwned>(&self, cmd: &str, args: serde_json::Value) -> Result<T, String> {
        self.0.run_mobile_plugin(cmd, args).map_err(|e| e.to_string())
    }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("gamerepo-device")
        .setup(|app, api| {
            #[cfg(target_os = "android")]
            {
                use tauri::Manager;
                app.manage(Android(api.register_android_plugin("io.github.drflgd.gamerepo.go.device", "DevicePlugin")?));
            }
            #[cfg(not(target_os = "android"))]
            let _ = (app, api);
            Ok(())
        })
        .build()
}
