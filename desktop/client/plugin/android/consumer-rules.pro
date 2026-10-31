# Tauri keeps what its plugins need (its own consumer rules)
# The saves' job (WorkManager makes it by its name) and the Rust library's function for it
-keep class io.github.drflgd.gamerepo.go.device.SavesWorker { <init>(...); }
-keep class io.github.drflgd.gamerepo.go.device.SavesNative { native <methods>; }
# WorkManager's Room database, made by its name with no arguments (R8 had taken the
# constructor away, and the app crashed as it started)
-keep class * extends androidx.room.RoomDatabase { <init>(); }
