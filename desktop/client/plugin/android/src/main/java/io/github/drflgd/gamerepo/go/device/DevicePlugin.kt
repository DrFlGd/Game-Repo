// Game Repo Go's Android side (docs/PLAN.md, "Phase 9b design", D97): what only Android can
// do, asked for by the Rust shell (desktop/client/src/android.rs) with run_mobile_plugin,
// never by the page. All files access (D87), the device's storage volumes (C3, C9), a folder
// chosen with Android's own picker, the pairing token in the Android Keystore, the multicast
// lock while the library is looked for, the downloads' foreground service and its
// notification (C10), controllers turned into keys (Controllers.kt), Game Repo Go's updates
// (Updates.kt, D139), and the saves' job in the background (SavesWorker.kt, D186).
package io.github.drflgd.gamerepo.go.device

import android.Manifest
import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Environment
import android.provider.DocumentsContract
import android.provider.Settings
import android.webkit.WebView
import androidx.activity.result.ActivityResult
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
class NameArgs {
    var name: String = ""
}

@InvokeArg
class SecretArgs {
    var name: String = ""
    var value: String = ""
}

@InvokeArg
class OnArgs {
    var on: Boolean = false
}

@InvokeArg
class DownloadsArgs {
    var running: Boolean = false
    var title: String = ""
    var text: String = ""
    // 0 to 100, or -1 while there's nothing to measure yet
    var percent: Int = -1
    var paused: Boolean = false
}

@InvokeArg
class SavesArgs {
    var on: Boolean = false
    var data: String = ""
    var minutes: Int = 15
}

@InvokeArg
class UpdateArgs {
    var url: String = ""
    var path: String = ""
}

@TauriPlugin
class DevicePlugin(private val activity: Activity) : Plugin(activity) {
    private var multicast: WifiManager.MulticastLock? = null
    private var askedNotifications = false

    override fun load(webView: WebView) {
        Controllers.install(activity, webView)
        // the page has the keys from the start, as on a handheld nothing may ever be touched
        webView.post { if (!webView.hasFocus()) webView.requestFocus() }
    }

    private fun hasStorage(): Boolean =
        if (Build.VERSION.SDK_INT >= 30) Environment.isExternalStorageManager()
        else activity.checkSelfPermission(Manifest.permission.WRITE_EXTERNAL_STORAGE) == PackageManager.PERMISSION_GRANTED

    private fun granted(invoke: Invoke) = invoke.resolve(JSObject().apply { put("granted", hasStorage()) })

    /** Whether Game Repo Go may write ES-DE's folders. */
    @Command
    fun storageAccess(invoke: Invoke) = granted(invoke)

    /** Android's own screen for it (Android 11 and later: "Allow access to manage all files";
     *  Android 10: the storage permission), answered once the person comes back. */
    @Command
    fun askStorageAccess(invoke: Invoke) {
        if (hasStorage()) return granted(invoke)
        if (Build.VERSION.SDK_INT < 30) {
            activity.requestPermissions(arrayOf(Manifest.permission.READ_EXTERNAL_STORAGE, Manifest.permission.WRITE_EXTERNAL_STORAGE), 7)
            return granted(invoke)
        }
        val mine = Intent(Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION, Uri.parse("package:${activity.packageName}"))
        try {
            startActivityForResult(invoke, mine, "storageAsked")
        } catch (e: ActivityNotFoundException) {
            // some devices only have the list of every app
            try {
                startActivityForResult(invoke, Intent(Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION), "storageAsked")
            } catch (e: ActivityNotFoundException) {
                invoke.reject("This device has no screen for allowing access to all files.")
            }
        }
    }

    @ActivityCallback
    private fun storageAsked(invoke: Invoke, result: ActivityResult) = granted(invoke)

    /** The device's storage that's there now: internal storage and SD cards, each with
     *  its folder, as ES-DE's folder can be on any of them. */
    @Command
    fun volumes(invoke: Invoke) {
        val list = JSArray()
        for (v in Volumes.list(activity)) {
            list.put(JSObject().apply {
                put("path", v.path)
                put("primary", v.primary)
                put("removable", v.removable)
                put("label", v.label)
            })
        }
        invoke.resolve(JSObject().apply { put("volumes", list) })
    }

    /** A folder chosen with Android's own picker, as a path (ES-DE's folder or the ROMs
     *  folder, in Settings). An empty path if cancelled. */
    @Command
    fun pickFolder(invoke: Invoke) {
        try {
            startActivityForResult(invoke, Intent(Intent.ACTION_OPEN_DOCUMENT_TREE), "folderPicked")
        } catch (e: ActivityNotFoundException) {
            invoke.reject("This device has no folder picker: type the folder instead.")
        }
    }

    @ActivityCallback
    private fun folderPicked(invoke: Invoke, result: ActivityResult) {
        val uri = result.data?.data
        invoke.resolve(JSObject().apply { put("path", if (result.resultCode == Activity.RESULT_OK && uri != null) treePath(uri) ?: "" else "") })
    }

    /** content://com.android.externalstorage.documents/tree/primary%3AES-DE is
     *  /storage/emulated/0/ES-DE; …/tree/1234-5678%3AROMs is /storage/1234-5678/ROMs. */
    private fun treePath(uri: Uri): String? {
        if (uri.authority != "com.android.externalstorage.documents") return null
        val id = DocumentsContract.getTreeDocumentId(uri)
        val volume = id.substringBefore(':')
        val rest = id.substringAfter(':', "")
        val root = if (volume == "primary") Environment.getExternalStorageDirectory().absolutePath else "/storage/$volume"
        return if (rest.isEmpty()) root else "$root/$rest"
    }

    @Command
    fun secretGet(invoke: Invoke) {
        val a = invoke.parseArgs(NameArgs::class.java)
        try {
            val value = Secrets.get(activity, a.name)
            invoke.resolve(JSObject().apply {
                put("found", value != null)
                put("value", value ?: "")
            })
        } catch (e: Exception) {
            invoke.reject("The pairing can't be read from Android's Keystore: ${e.message}")
        }
    }

    @Command
    fun secretSet(invoke: Invoke) {
        val a = invoke.parseArgs(SecretArgs::class.java)
        try {
            Secrets.set(activity, a.name, a.value)
            invoke.resolve()
        } catch (e: Exception) {
            invoke.reject("The pairing can't be kept in Android's Keystore: ${e.message}")
        }
    }

    @Command
    fun secretDelete(invoke: Invoke) {
        Secrets.delete(activity, invoke.parseArgs(NameArgs::class.java).name)
        invoke.resolve()
    }

    /** Android drops multicast to save power unless an app holds this while it looks for
     *  the library on the network (DNS-SD). */
    @Command
    fun multicast(invoke: Invoke) {
        val on = invoke.parseArgs(OnArgs::class.java).on
        synchronized(this) {
            if (on && multicast == null) {
                val wifi = activity.applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
                multicast = wifi.createMulticastLock("Game Repo Go").apply {
                    setReferenceCounted(false)
                    acquire()
                }
            } else if (!on) {
                multicast?.release()
                multicast = null
            }
        }
        invoke.resolve()
    }

    /** While the queue runs: the foreground service, with its notification saying how far it
     *  is, so downloads go on with the screen off or another app open (C10). */
    @Command
    fun downloads(invoke: Invoke) {
        val a = invoke.parseArgs(DownloadsArgs::class.java)
        val ctx = activity.applicationContext
        if (!a.running) {
            DownloadService.stop(ctx)
            return invoke.resolve()
        }
        if (Build.VERSION.SDK_INT >= 33 && !askedNotifications && ctx.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            askedNotifications = true
            activity.requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 8)
        }
        try {
            DownloadService.show(ctx, DownloadService.Shown(a.title, a.text, a.percent, a.paused))
            invoke.resolve()
        } catch (e: Exception) {
            // Android won't start one while Game Repo Go is in the background: the queue
            // goes on while it's open, and the service starts when it's next in front
            invoke.reject("The downloads' notification can't be shown: ${e.message}")
        }
    }

    /** Saves synced with Game Repo Go closed (D186): WorkManager's job for Go's data folder
     *  set up, or taken away. */
    @Command
    fun savesBackground(invoke: Invoke) {
        val a = invoke.parseArgs(SavesArgs::class.java)
        SavesWorker.set(activity.applicationContext, a.on, a.data, a.minutes.toLong())
        invoke.resolve()
    }

    /** Waits for Pause, Resume or Cancel tapped on the notification (one at a time; the Rust
     *  side asks again after each). */
    @Command
    fun waitAction(invoke: Invoke) = Actions.wait(invoke)

    /** Game Repo Go's updates (D139, Updates.kt): the update file, the APK and how far it's
     *  come, and Android's installer for it. */
    @Command
    fun updateFetch(invoke: Invoke) {
        Updates.fetch(invoke, invoke.parseArgs(UpdateArgs::class.java).url)
    }

    @Command
    fun updateDownload(invoke: Invoke) {
        Updates.download(activity, invoke, invoke.parseArgs(UpdateArgs::class.java).url)
    }

    @Command
    fun updateProgress(invoke: Invoke) = Updates.progress(invoke)

    @Command
    fun updateInstall(invoke: Invoke) {
        Updates.install(activity, invoke, invoke.parseArgs(UpdateArgs::class.java).path)
    }

    /** Game Repo Go to the background (Back on the Library with nothing to go back from). */
    @Command
    fun moveToBack(invoke: Invoke) {
        activity.moveTaskToBack(true)
        invoke.resolve()
    }
}

/** The notification's buttons, handed to whoever waits for them. */
object Actions {
    private var waiting: Invoke? = null
    private val queued = ArrayDeque<String>()

    @Synchronized
    fun tapped(action: String) {
        val w = waiting
        if (w != null) {
            waiting = null
            w.resolve(JSObject().apply { put("action", action) })
        } else if (queued.size < 8) {
            queued.addLast(action)
        }
    }

    @Synchronized
    fun wait(invoke: Invoke) {
        val next = queued.removeFirstOrNull()
        if (next != null) return invoke.resolve(JSObject().apply { put("action", next) })
        // a second wait replaces the first, which hears nothing
        waiting?.resolve(JSObject().apply { put("action", "") })
        waiting = invoke
    }
}
