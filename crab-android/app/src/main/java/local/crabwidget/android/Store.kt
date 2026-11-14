package local.crabwidget.android

import android.content.Context
import org.json.JSONArray
import org.json.JSONObject

/** Everything the app keeps between runs, in one small SharedPreferences file. */
object Store {
    /** Choices for how often usage is checked while the app is open, in seconds. */
    val INTERVALS = listOf(120L to "2 minutes", 300L to "5 minutes", 900L to "15 minutes")

    private const val FILE = "crab"
    private fun prefs(c: Context) = c.getSharedPreferences(FILE, Context.MODE_PRIVATE)

    private fun obj(c: Context, key: String): JSONObject? =
        prefs(c).getString(key, null)?.let { runCatching { JSONObject(it) }.getOrNull() }

    /** The latest check, good or not: {state, data?, status?, at}. */
    fun latest(c: Context): JSONObject = obj(c, "latest") ?: JSONObject().put("state", "waiting")
    /** The latest check that returned usage data. */
    fun good(c: Context): JSONObject? = obj(c, "good")
    fun chats(c: Context): JSONObject? = obj(c, "chats")
    fun history(c: Context): JSONArray =
        prefs(c).getString("history", null)?.let { runCatching { JSONArray(it) }.getOrNull() } ?: JSONArray()

    fun saveLatest(c: Context, msg: JSONObject) = prefs(c).edit().putString("latest", msg.toString()).apply()
    fun saveGood(c: Context, msg: JSONObject) = prefs(c).edit().putString("good", msg.toString()).apply()
    fun saveChats(c: Context, msg: JSONObject?) = prefs(c).edit().putString("chats", msg?.toString()).apply()
    fun saveHistory(c: Context, list: JSONArray) = prefs(c).edit().putString("history", list.toString()).apply()

    fun lastPct(c: Context): Double? =
        prefs(c).getFloat("last_pct", -1f).takeIf { it >= 0f }?.toDouble()
    fun saveLastPct(c: Context, pct: Double) = prefs(c).edit().putFloat("last_pct", pct.toFloat()).apply()

    /* ---------- updates ---------- */

    fun update(c: Context): JSONObject? = obj(c, "update")
    fun saveUpdate(c: Context, json: JSONObject) = prefs(c).edit().putString("update", json.toString()).apply()
    fun lastUpdateCheck(c: Context): Long = prefs(c).getLong("update_checked", 0L)
    fun saveLastUpdateCheck(c: Context, t: Long) = prefs(c).edit().putLong("update_checked", t).apply()
    fun notifiedUpdate(c: Context): Long = prefs(c).getLong("update_notified", 0L)
    fun saveNotifiedUpdate(c: Context, code: Long) = prefs(c).edit().putLong("update_notified", code).apply()

    /* ---------- settings ---------- */

    fun flag(c: Context, key: String): Boolean = prefs(c).getBoolean(key, DEFAULTS[key] ?: false)
    fun setFlag(c: Context, key: String, on: Boolean) = prefs(c).edit().putBoolean(key, on).apply()
    fun interval(c: Context): Long = prefs(c).getLong("interval", 300L)
    fun setInterval(c: Context, secs: Long) {
        if (INTERVALS.any { it.first == secs }) prefs(c).edit().putLong("interval", secs).apply()
    }

    private val DEFAULTS = mapOf(
        "remaining" to false,   // show what is left instead of what is used
        "smooth" to false,      // glide between pixel poses
        "chats" to true,        // list recent claude.ai chats and their status
        "background" to true,   // check every 15 minutes while the app is closed
        "notify" to true,       // alerts at 75%, 90% and on reset
    )

    fun settings(c: Context): JSONObject = JSONObject().apply {
        for (key in DEFAULTS.keys) put(key, flag(c, key))
        put("interval", interval(c))
        put("intervals", JSONArray(INTERVALS.map { JSONArray().put(it.first).put(it.second) }))
    }
}
