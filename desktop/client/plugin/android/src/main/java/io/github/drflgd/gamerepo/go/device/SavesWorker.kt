// Saves synced with Game Repo Go closed, on Android (docs/PLAN.md, D186, "Save sync notes,
// part 6"): a job of WorkManager's, about every fifteen minutes while the network's there,
// though Android puts background work off when it saves the battery (Doze, an app it thinks
// rarely used). Set up by the Rust shell (the plugin's `savesBackground`) while Go's paired
// and Settings › Saves says so, and taken away when not. The job unseals the pairing here
// (Secrets.kt) and syncs through Go's own Rust library, loaded into the process when the
// app's window isn't open (desktop/client/src/android.rs, `sync_alone`); a sync already
// going on in the app makes it wait for the next time.
package io.github.drflgd.gamerepo.go.device

import android.content.Context
import android.util.Log
import androidx.work.Constraints
import androidx.work.Data
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.NetworkType
import androidx.work.PeriodicWorkRequest
import androidx.work.WorkManager
import androidx.work.Worker
import androidx.work.WorkerParameters
import org.json.JSONObject
import java.util.concurrent.TimeUnit

class SavesWorker(ctx: Context, params: WorkerParameters) : Worker(ctx, params) {
    companion object {
        private const val NAME = "Game Repo Go saves"
        private const val TAG = "GameRepoGoSaves"

        /** The job on, for Go's data folder, or off. */
        fun set(ctx: Context, on: Boolean, data: String, minutes: Long) {
            val work = WorkManager.getInstance(ctx)
            if (!on) {
                work.cancelUniqueWork(NAME)
                return
            }
            val request = PeriodicWorkRequest.Builder(SavesWorker::class.java, minutes.coerceAtLeast(15), TimeUnit.MINUTES)
                .setConstraints(Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).setRequiresBatteryNotLow(true).build())
                .setInputData(Data.Builder().putString("data", data).build())
                .build()
            // (UPDATE keeps its next time when Go starts again and sets it up once more)
            work.enqueueUniquePeriodicWork(NAME, ExistingPeriodicWorkPolicy.UPDATE, request)
        }
    }

    override fun doWork(): Result {
        val ctx = applicationContext
        val data = inputData.getString("data") ?: return Result.success()
        val server = try {
            Secrets.get(ctx, "server")
        } catch (e: Exception) {
            Log.w(TAG, "the pairing couldn't be unsealed", e)
            null
        } ?: return Result.success()
        return try {
            System.loadLibrary("gamerepo_client_lib")
            val said = JSONObject(SavesNative.sync(data, Volumes.list(ctx).joinToString("\n") { it.path }, server) ?: "{}")
            // the library found at another address: kept, as the app would
            if (!said.isNull("server")) Secrets.set(ctx, "server", said.getString("server"))
            Log.i(TAG, "synced: " + (if (said.isNull("summary")) "nothing to sync" else said.get("summary").toString()) +
                (if (said.isNull("error")) "" else " (${said.get("error")})"))
            Result.success()
        } catch (e: Throwable) {
            Log.w(TAG, "the saves couldn't be synced", e)
            Result.success()
        }
    }
}

/** Go's Rust library (libgamerepo_client_lib.so): one sync of saves, answered as JSON (the
 *  sync's summary, an error, and the pairing again if it changed). */
object SavesNative {
    @JvmStatic
    external fun sync(data: String, volumes: String, server: String): String?
}
