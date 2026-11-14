package local.crabwidget.android

import android.Manifest
import android.app.ActivityManager
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.os.BatteryManager
import android.os.Build
import android.app.Notification
import org.json.JSONArray
import org.json.JSONObject

/** Turns probe answers into the stored state, the widget and the limit alerts. */
object Usage {
    const val CHANNEL = "limits"
    private const val HISTORY_MS = 60 * 60 * 1000L

    fun now() = System.currentTimeMillis()

    /** Stores one probe answer. Returns whether usage data came back. */
    fun record(c: Context, payload: JSONObject): Boolean {
        val ok = payload.optBoolean("ok")
        val msg = if (ok) {
            val data = payload.optJSONObject("usage") ?: JSONObject()
            JSONObject().put("state", "ok").put("data", data).put("at", now())
        } else {
            val status = payload.optInt("status", -1)
            val state = if (status == 401 || status == 403) "login" else "error"
            JSONObject().put("state", state).put("status", status).put("at", now())
        }
        Store.saveLatest(c, msg)
        if (ok) {
            Store.saveGood(c, msg)
            sessionPct(msg)?.let { pct ->
                addHistory(c, pct)
                alertThresholds(c, pct)
            }
        }
        CrabWidget.refresh(c)
        return ok
    }

    fun recordChats(c: Context, payload: JSONObject?) {
        if (payload == null || !payload.optBoolean("ok")) return
        Store.saveChats(c, JSONObject().put("list", payload.optJSONArray("chats") ?: JSONArray()).put("at", now()))
    }

    fun pctOf(bucket: JSONObject?): Double? =
        bucket?.opt("utilization")?.let { (it as? Number)?.toDouble() }

    fun sessionPct(msg: JSONObject?): Double? = pctOf(msg?.optJSONObject("data")?.optJSONObject("five_hour"))
    fun weekPct(msg: JSONObject?): Double? = pctOf(msg?.optJSONObject("data")?.optJSONObject("seven_day"))

    private fun addHistory(c: Context, pct: Double) {
        val t = now()
        val old = Store.history(c)
        val out = JSONArray()
        // A drop means the session reset: the forecast starts over.
        val last = if (old.length() > 0) old.getJSONObject(old.length() - 1).optDouble("pct") else null
        if (last == null || pct >= last - 1) {
            for (i in 0 until old.length()) {
                val s = old.getJSONObject(i)
                if (t - s.optLong("t") <= HISTORY_MS) out.put(s)
            }
        }
        out.put(JSONObject().put("t", t).put("pct", pct))
        Store.saveHistory(c, out)
    }

    /* ---------- alerts ---------- */

    fun createChannel(c: Context) {
        val nm = c.getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(
            NotificationChannel(CHANNEL, c.getString(R.string.channel_limits), NotificationManager.IMPORTANCE_DEFAULT)
        )
    }

    private fun alertThresholds(c: Context, pct: Double) {
        val prev = Store.lastPct(c)
        Store.saveLastPct(c, pct)
        if (prev == null || !Store.flag(c, "notify")) return
        val p = pct.toInt()
        when {
            prev < 90 && pct >= 90 -> notify(c, 1, "Claude limit almost reached", "Your 5-hour session is $p% used.")
            prev < 75 && pct >= 75 -> notify(c, 1, "Claude limit at 75%", "Your 5-hour session is $p% used.")
            prev >= 75 && pct < 10 -> notify(c, 2, "Claude limit has reset", "Your 5-hour session starts fresh.")
        }
    }

    private fun notify(c: Context, id: Int, title: String, body: String) {
        if (Build.VERSION.SDK_INT >= 33 &&
            c.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) return
        val open = PendingIntent.getActivity(
            c, 0, Intent(c, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        )
        val n = Notification.Builder(c, CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_crab)
            .setContentTitle(title)
            .setContentText(body)
            .setContentIntent(open)
            .setAutoCancel(true)
            .build()
        c.getSystemService(NotificationManager::class.java).notify(id, n)
    }

    /* ---------- the phone itself: RAM and battery (Android hides CPU load from apps) ---------- */

    fun phone(c: Context): JSONObject {
        val am = c.getSystemService(ActivityManager::class.java)
        val mem = ActivityManager.MemoryInfo().also { am.getMemoryInfo(it) }
        val battery = c.registerReceiver(null, IntentFilter(Intent.ACTION_BATTERY_CHANGED))
        val level = battery?.getIntExtra(BatteryManager.EXTRA_LEVEL, -1) ?: -1
        val scale = battery?.getIntExtra(BatteryManager.EXTRA_SCALE, 100) ?: 100
        val temp = battery?.getIntExtra(BatteryManager.EXTRA_TEMPERATURE, -1) ?: -1
        val plugged = (battery?.getIntExtra(BatteryManager.EXTRA_PLUGGED, 0) ?: 0) != 0
        return JSONObject()
            .put("memUsed", mem.totalMem - mem.availMem)
            .put("memTotal", mem.totalMem)
            .put("battery", if (level >= 0 && scale > 0) level * 100.0 / scale else JSONObject.NULL)
            .put("temp", if (temp > 0) temp / 10.0 else JSONObject.NULL)
            .put("charging", plugged)
    }
}
