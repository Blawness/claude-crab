package local.crabwidget.android

import android.content.Context
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.NetworkType
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import java.util.concurrent.TimeUnit

/** Checks usage every 15 minutes (Android's shortest period) while the app is closed. */
class CrabWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result {
        val probe = ClaudeProbe(applicationContext)
        try {
            Usage.record(applicationContext, probe.usage())
        } finally {
            probe.destroy()
        }
        return Result.success()
    }

    companion object {
        private const val NAME = "usage-check"

        /** Starts or stops the background check to match the setting. */
        fun sync(c: Context) {
            val wm = WorkManager.getInstance(c)
            if (!Store.flag(c, "background")) {
                wm.cancelUniqueWork(NAME)
                return
            }
            val request = PeriodicWorkRequestBuilder<CrabWorker>(15, TimeUnit.MINUTES)
                .setConstraints(Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build())
                .build()
            wm.enqueueUniquePeriodicWork(NAME, ExistingPeriodicWorkPolicy.KEEP, request)
        }
    }
}
