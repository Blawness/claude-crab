package local.crabwidget.android

import android.app.Notification
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInfo
import android.content.pm.PackageInstaller
import android.content.pm.PackageManager
import android.Manifest
import android.os.Build
import androidx.core.content.pm.PackageInfoCompat
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest

/**
 * Updates from the "android-latest" GitHub release. The release carries version.json (build number
 * and the APK's sha256) next to the APK. Android still shows its own install confirmation; the APK
 * must also be signed with the same key as the installed app, or Android refuses it.
 */
object Updater {
    private const val BASE = "https://github.com/Blawness/claude-crab/releases/download/android-latest/"
    private const val VERSION_URL = BASE + "version.json"
    private const val APK_URL = BASE + "ClaudeCrab.apk"
    private const val CHECK_EVERY_MS = 6 * 60 * 60 * 1000L
    private const val NOTIFY_ID = 10
    const val ACTION_INSTALLED = "local.crabwidget.android.INSTALL_STATUS"

    data class Release(val code: Long, val name: String, val sha256: String)

    private fun info(c: Context): PackageInfo = c.packageManager.getPackageInfo(c.packageName, 0)
    fun currentCode(c: Context): Long = PackageInfoCompat.getLongVersionCode(info(c))
    fun currentName(c: Context): String = info(c).versionName ?: "?"

    /** The newer release, if the last check found one. */
    fun pending(c: Context): Release? {
        val j = Store.update(c) ?: return null
        val r = Release(j.optLong("versionCode"), j.optString("versionName"), j.optString("sha256"))
        return r.takeIf { it.code > currentCode(c) }
    }

    /** Checks at most every few hours unless [force]. Returns the newer release, if any. */
    suspend fun check(c: Context, force: Boolean = false, notify: Boolean = false): Release? {
        if (!force && Usage.now() - Store.lastUpdateCheck(c) < CHECK_EVERY_MS) return pending(c)
        val json = withContext(Dispatchers.IO) {
            runCatching { JSONObject(String(get(VERSION_URL).use { it.readBytes() })) }.getOrNull()
        } ?: return pending(c)
        Store.saveUpdate(c, json)
        Store.saveLastUpdateCheck(c, Usage.now())
        val newer = pending(c)
        if (newer != null && notify && Store.notifiedUpdate(c) < newer.code) {
            Store.saveNotifiedUpdate(c, newer.code)
            notifyAvailable(c, newer)
        }
        return newer
    }

    private fun get(url: String) = (URL(url).openConnection() as HttpURLConnection).run {
        connectTimeout = 15_000
        readTimeout = 30_000
        instanceFollowRedirects = true
        setRequestProperty("User-Agent", "ClaudeCrab-Android")
        if (responseCode !in 200..299) throw IllegalStateException("HTTP $responseCode")
        inputStream
    }

    /** Downloads the APK and checks it against the sha256 in version.json. */
    suspend fun download(c: Context, r: Release, progress: (Int) -> Unit): File = withContext(Dispatchers.IO) {
        val dir = File(c.cacheDir, "updates").apply { deleteRecursively(); mkdirs() }
        val file = File(dir, "ClaudeCrab-${r.code}.apk")
        val conn = URL(APK_URL).openConnection() as HttpURLConnection
        conn.connectTimeout = 15_000
        conn.readTimeout = 30_000
        conn.setRequestProperty("User-Agent", "ClaudeCrab-Android")
        if (conn.responseCode !in 200..299) throw IllegalStateException("Download failed (HTTP ${conn.responseCode})")
        val total = conn.contentLengthLong
        val digest = MessageDigest.getInstance("SHA-256")
        conn.inputStream.use { input ->
            file.outputStream().use { out ->
                val buf = ByteArray(64 * 1024)
                var done = 0L
                var last = -1
                while (true) {
                    val n = input.read(buf)
                    if (n < 0) break
                    out.write(buf, 0, n)
                    digest.update(buf, 0, n)
                    done += n
                    val pct = if (total > 0) (done * 100 / total).toInt() else -1
                    if (pct != last) {
                        last = pct
                        withContext(Dispatchers.Main) { progress(pct) }
                    }
                }
            }
        }
        val hash = digest.digest().joinToString("") { "%02x".format(it) }
        if (r.sha256.isNotEmpty() && !hash.equals(r.sha256, ignoreCase = true)) {
            file.delete()
            throw IllegalStateException("Downloaded file does not match the release")
        }
        file
    }

    /** Whether Android lets this app install APKs ("Install unknown apps" for Claude Crab). */
    fun canInstall(c: Context) = c.packageManager.canRequestPackageInstalls()

    /** Hands the APK to Android's installer. The result comes back to [InstallReceiver]. */
    suspend fun install(c: Context, apk: File) = withContext(Dispatchers.IO) {
        val installer = c.packageManager.packageInstaller
        val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL).apply {
            setAppPackageName(c.packageName)
            // Android 12+: may skip the confirmation once this app is its own installer of record.
            if (Build.VERSION.SDK_INT >= 31) setRequireUserAction(PackageInstaller.SessionParams.USER_ACTION_NOT_REQUIRED)
        }
        val id = installer.createSession(params)
        installer.openSession(id).use { session ->
            session.openWrite("base.apk", 0, apk.length()).use { out ->
                apk.inputStream().use { it.copyTo(out) }
                session.fsync(out)
            }
            val flags = PendingIntent.FLAG_UPDATE_CURRENT or
                if (Build.VERSION.SDK_INT >= 31) PendingIntent.FLAG_MUTABLE else 0
            val intent = Intent(c, InstallReceiver::class.java).setAction(ACTION_INSTALLED)
            val sender = PendingIntent.getBroadcast(c, id, intent, flags).intentSender
            session.commit(sender)
        }
    }

    private fun notifyAvailable(c: Context, r: Release) {
        if (Build.VERSION.SDK_INT >= 33 &&
            c.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) return
        val open = PendingIntent.getActivity(
            c, 1, Intent(c, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val n = Notification.Builder(c, Usage.CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_crab)
            .setContentTitle("Claude Crab update available")
            .setContentText("Build ${r.code} is ready. Tap to update.")
            .setContentIntent(open)
            .setAutoCancel(true)
            .build()
        c.getSystemService(NotificationManager::class.java).notify(NOTIFY_ID, n)
    }
}

/** Android's answer to an install: ask the user to confirm, or report a failure. */
class InstallReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        when (intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE)) {
            PackageInstaller.STATUS_PENDING_USER_ACTION -> {
                @Suppress("DEPRECATION")
                val confirm = if (Build.VERSION.SDK_INT >= 33)
                    intent.getParcelableExtra(Intent.EXTRA_INTENT, Intent::class.java)
                else intent.getParcelableExtra(Intent.EXTRA_INTENT)
                confirm?.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)?.let { runCatching { context.startActivity(it) } }
            }
            PackageInstaller.STATUS_SUCCESS -> Unit // this process is replaced by the new version
            else -> {
                val why = intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE) ?: "unknown reason"
                MainActivity.installFailed(why)
            }
        }
    }
}
