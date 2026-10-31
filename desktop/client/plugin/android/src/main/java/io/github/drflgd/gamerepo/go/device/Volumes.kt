// The device's storage volumes that are there now (C3, C9): internal storage and SD cards,
// each with its folder, as ES-DE's folder and the emulators' saves can be on any of them. For
// the plugin's `volumes` and the saves' job in the background (SavesWorker.kt).
package io.github.drflgd.gamerepo.go.device

import android.content.Context
import android.os.Build
import android.os.Environment
import android.os.storage.StorageManager

object Volumes {
    data class Volume(val path: String, val primary: Boolean, val removable: Boolean, val label: String)

    fun list(ctx: Context): List<Volume> {
        val list = ArrayList<Volume>()
        val seen = HashSet<String>()
        fun add(path: String, primary: Boolean, removable: Boolean, label: String) {
            if (seen.add(path)) list.add(Volume(path, primary, removable, label))
        }
        if (Build.VERSION.SDK_INT >= 30) {
            val sm = ctx.getSystemService(Context.STORAGE_SERVICE) as StorageManager
            for (v in sm.storageVolumes) {
                val dir = v.directory ?: continue
                if (v.state != Environment.MEDIA_MOUNTED) continue
                add(dir.absolutePath, v.isPrimary, v.isRemovable, v.getDescription(ctx))
            }
        } else {
            // Android 10: each volume's app folder, <volume>/Android/data/<id>/files
            for ((i, f) in ctx.getExternalFilesDirs(null).withIndex()) {
                if (f == null) continue
                add(f.absolutePath.substringBefore("/Android/"), i == 0, i > 0, if (i == 0) "Internal storage" else "SD card")
            }
        }
        return list
    }
}
