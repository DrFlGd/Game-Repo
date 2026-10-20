// The downloads' foreground service (D97, C10): Android's usual notification while the queue
// runs ("Downloading 3 games, 45%", with Pause or Resume and Cancel), a Wi-Fi lock so the
// connection doesn't sleep, and a wake lock so the downloads don't stop with the screen.
// The queue itself is the Rust side's; this only keeps the app running and shows it.
// Android 15 lets such a service run six hours a day: past that it stops, and the queue goes
// on when Game Repo Go is next opened.
package io.github.drflgd.gamerepo.go.device

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.wifi.WifiManager
import android.os.Build
import android.os.IBinder
import android.os.PowerManager

class DownloadService : Service() {
    data class Shown(val title: String, val text: String, val percent: Int, val paused: Boolean)

    companion object {
        private const val CHANNEL = "downloads"
        private const val ID = 1
        @Volatile private var running: DownloadService? = null
        @Volatile private var latest = Shown("Game Repo Go", "", -1, false)

        /** Starts the service, or updates its notification while it runs. */
        fun show(ctx: Context, shown: Shown) {
            latest = shown
            val service = running
            if (service != null) {
                service.update()
                return
            }
            val intent = Intent(ctx, DownloadService::class.java)
            if (Build.VERSION.SDK_INT >= 26) ctx.startForegroundService(intent) else ctx.startService(intent)
        }

        fun stop(ctx: Context) {
            if (running != null) ctx.stopService(Intent(ctx, DownloadService::class.java))
        }
    }

    private var wifi: WifiManager.WifiLock? = null
    private var wake: PowerManager.WakeLock? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        val nm = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        if (Build.VERSION.SDK_INT >= 26) {
            nm.createNotificationChannel(NotificationChannel(CHANNEL, "Downloads", NotificationManager.IMPORTANCE_LOW).apply {
                description = "Games downloading from your library"
                setShowBadge(false)
            })
        }
        @Suppress("DEPRECATION")
        val mode = if (Build.VERSION.SDK_INT >= 29) WifiManager.WIFI_MODE_FULL_HIGH_PERF else WifiManager.WIFI_MODE_FULL
        wifi = (applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager).createWifiLock(mode, "Game Repo Go downloads").apply {
            setReferenceCounted(false)
            acquire()
        }
        wake = (getSystemService(Context.POWER_SERVICE) as PowerManager).newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "GameRepoGo:downloads").apply {
            setReferenceCounted(false)
            acquire()
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            "pause", "resume", "cancel" -> Actions.tapped(intent.action!!)
        }
        val n = build()
        if (Build.VERSION.SDK_INT >= 29) startForeground(ID, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC) else startForeground(ID, n)
        running = this
        return START_NOT_STICKY
    }

    // Android 15's six hours a day for this kind of service are up
    override fun onTimeout(startId: Int, fgsType: Int) {
        stopSelf()
    }

    override fun onDestroy() {
        running = null
        wifi?.release()
        wake?.release()
        if (Build.VERSION.SDK_INT >= 24) stopForeground(STOP_FOREGROUND_REMOVE) else @Suppress("DEPRECATION") stopForeground(true)
        super.onDestroy()
    }

    fun update() {
        (getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager).notify(ID, build())
    }

    private fun action(name: String, label: String, code: Int): Notification.Action {
        val intent = Intent(this, DownloadService::class.java).setAction(name)
        val pending = PendingIntent.getService(this, code, intent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        @Suppress("DEPRECATION")
        return Notification.Action.Builder(0, label, pending).build()
    }

    private fun build(): Notification {
        val s = latest
        val b = if (Build.VERSION.SDK_INT >= 26) Notification.Builder(this, CHANNEL) else @Suppress("DEPRECATION") Notification.Builder(this)
        val open = packageManager.getLaunchIntentForPackage(packageName)?.let {
            PendingIntent.getActivity(this, 0, it, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        }
        b.setSmallIcon(if (s.paused) android.R.drawable.stat_sys_download_done else android.R.drawable.stat_sys_download)
            .setContentTitle(s.title)
            .setContentText(s.text)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setShowWhen(false)
            .setCategory(Notification.CATEGORY_PROGRESS)
            .setContentIntent(open)
            .addAction(if (s.paused) action("resume", "Resume", 1) else action("pause", "Pause", 1))
            .addAction(action("cancel", "Cancel", 2))
        if (!s.paused) b.setProgress(100, s.percent.coerceIn(0, 100), s.percent < 0)
        if (Build.VERSION.SDK_INT >= 31) b.setForegroundServiceBehavior(Notification.FOREGROUND_SERVICE_IMMEDIATE)
        return b.build()
    }
}
