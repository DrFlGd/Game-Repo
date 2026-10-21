// Game Repo Go's updates on Android (docs/PLAN.md, "In-app updates", D139): the update file
// and the APK fetched with Android's own HTTPS (GitHub's address sends it on to where the
// file is), and the APK handed to Android's package installer, which asks the person, checks
// it's signed with the same key as the copy installed, and replaces Game Repo Go. The Rust
// side (desktop/client/src/android_updates.rs) checks the APK against the key Game Repo Go
// was built with before it gets here.
package io.github.drflgd.gamerepo.go.device

import android.app.Activity
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageInstaller
import android.os.Build
import androidx.core.content.ContextCompat
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import java.io.File
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URI
import kotlin.concurrent.thread

object Updates {
    private const val STATUS = "io.github.drflgd.gamerepo.go.UPDATE_STATUS"
    // the update file is small; anything bigger isn't one
    private const val MOST_TEXT = 4 shl 20

    @Volatile private var done = 0L
    @Volatile private var size = -1L
    @Volatile private var waiting: Invoke? = null
    private var listening = false

    private fun open(url: String, what: String): HttpURLConnection {
        val c = URI(url).toURL().openConnection() as HttpURLConnection
        c.connectTimeout = 20_000
        c.readTimeout = 60_000
        c.instanceFollowRedirects = true
        c.setRequestProperty("User-Agent", "Game Repo Go")
        val code = c.responseCode
        if (code !in 200..299) {
            c.disconnect()
            throw IOException(if (code == 404) "$what isn't in the newest release" else "GitHub answered $code for $what")
        }
        return c
    }

    private fun failed(e: Exception) = e.message ?: e.javaClass.simpleName

    /** The update file's text. */
    fun fetch(invoke: Invoke, url: String) = thread(name = "update-fetch") {
        try {
            val c = open(url, "the update file")
            val text = c.inputStream.use { input ->
                val bytes = input.readNBytesCompat(MOST_TEXT + 1)
                if (bytes.size > MOST_TEXT) throw IOException("the update file is too big")
                String(bytes, Charsets.UTF_8)
            }
            c.disconnect()
            invoke.resolve(JSObject().apply { put("text", text) })
        } catch (e: Exception) {
            invoke.reject(failed(e))
        }
    }

    /** The APK, into Game Repo Go's cache (any earlier one removed first). */
    fun download(activity: Activity, invoke: Invoke, url: String) = thread(name = "update-download") {
        val dir = File(activity.cacheDir, "update")
        try {
            dir.deleteRecursively()
            dir.mkdirs()
            val file = File(dir, "game-repo-go.apk")
            done = 0
            size = -1
            val c = open(url, "the APK")
            size = c.contentLengthLong
            c.inputStream.use { input ->
                file.outputStream().use { out ->
                    val buf = ByteArray(1 shl 16)
                    while (true) {
                        val n = input.read(buf)
                        if (n < 0) break
                        out.write(buf, 0, n)
                        done += n
                    }
                }
            }
            c.disconnect()
            invoke.resolve(JSObject().apply { put("path", file.absolutePath) })
        } catch (e: Exception) {
            dir.deleteRecursively()
            invoke.reject(failed(e))
        }
    }

    fun progress(invoke: Invoke) = invoke.resolve(JSObject().apply {
        put("done", done)
        put("size", size)
    })

    /** Android's installer for the APK at `path`: answered once it says it wasn't installed
     *  (installed, Android ends Game Repo Go to replace it). */
    fun install(activity: Activity, invoke: Invoke, path: String) = thread(name = "update-install") {
        try {
            val file = File(path)
            val installer = activity.packageManager.packageInstaller
            val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL)
            params.setAppPackageName(activity.packageName)
            val id = installer.createSession(params)
            installer.openSession(id).use { session ->
                session.openWrite("game-repo-go.apk", 0, file.length()).use { out ->
                    file.inputStream().use { it.copyTo(out) }
                    session.fsync(out)
                }
                listen(activity.applicationContext, invoke)
                val status = Intent(STATUS).setPackage(activity.packageName)
                val flags = PendingIntent.FLAG_UPDATE_CURRENT or (if (Build.VERSION.SDK_INT >= 31) PendingIntent.FLAG_MUTABLE else 0)
                session.commit(PendingIntent.getBroadcast(activity, id, status, flags).intentSender)
            }
        } catch (e: Exception) {
            invoke.reject("Android couldn't start installing it: ${failed(e)}")
        }
    }

    /** What Android's installer says: first that it needs the person to agree (its screen is
     *  shown), then how it went. Only Game Repo Go's own installs reach it. */
    @Synchronized
    private fun listen(app: Context, invoke: Invoke) {
        waiting?.reject("Another update was started.")
        waiting = invoke
        if (listening) return
        listening = true
        val receiver = object : BroadcastReceiver() {
            override fun onReceive(context: Context, intent: Intent) {
                val status = intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE)
                if (status == PackageInstaller.STATUS_PENDING_USER_ACTION) {
                    val ask: Intent? = if (Build.VERSION.SDK_INT >= 33) intent.getParcelableExtra(Intent.EXTRA_INTENT, Intent::class.java)
                        else @Suppress("DEPRECATION") intent.getParcelableExtra(Intent.EXTRA_INTENT)
                    if (ask != null) context.startActivity(ask.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                    return
                }
                val w = synchronized(this@Updates) { waiting.also { waiting = null } } ?: return
                when (status) {
                    PackageInstaller.STATUS_SUCCESS -> w.resolve(JSObject().apply { put("installed", true) })
                    PackageInstaller.STATUS_FAILURE_ABORTED -> w.reject("The update wasn't installed.")
                    PackageInstaller.STATUS_FAILURE_CONFLICT, PackageInstaller.STATUS_FAILURE_INCOMPATIBLE ->
                        w.reject("Android won't install it over this copy of Game Repo Go (it's signed with another key, or for another device). Install it from the releases page instead.")
                    PackageInstaller.STATUS_FAILURE_STORAGE -> w.reject("There isn't room on this device to install it.")
                    else -> w.reject("Android didn't install it: ${intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE) ?: "error $status"}")
                }
            }
        }
        ContextCompat.registerReceiver(app, receiver, IntentFilter(STATUS), ContextCompat.RECEIVER_NOT_EXPORTED)
    }
}

/** InputStream.readNBytes is Android 13's; this is the same for older ones. */
private fun java.io.InputStream.readNBytesCompat(most: Int): ByteArray {
    val out = java.io.ByteArrayOutputStream()
    val buf = ByteArray(1 shl 14)
    while (out.size() < most) {
        val n = read(buf, 0, minOf(buf.size, most - out.size()))
        if (n < 0) break
        out.write(buf, 0, n)
    }
    return out.toByteArray()
}
