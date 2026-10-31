# Tauri keeps what its plugins need (its own consumer rules)
# The saves' job (WorkManager makes it by its name) and the Rust library's function for it
-keep class io.github.drflgd.gamerepo.go.device.SavesWorker { <init>(...); }
-keep class io.github.drflgd.gamerepo.go.device.SavesNative { native <methods>; }
