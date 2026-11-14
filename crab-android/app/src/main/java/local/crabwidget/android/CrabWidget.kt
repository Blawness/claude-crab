package local.crabwidget.android

import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.widget.RemoteViews
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/** The home-screen crab: rings, mood and the two limits, redrawn after every check. */
class CrabWidget : AppWidgetProvider() {
    override fun onUpdate(context: Context, manager: AppWidgetManager, ids: IntArray) {
        render(context, manager, ids)
    }

    override fun onEnabled(context: Context) {
        CrabWorker.sync(context)
    }

    companion object {
        /** Data older than this shows grey rings: two background checks were missed. */
        private const val STALE_MS = 40 * 60 * 1000L

        fun refresh(c: Context) {
            val manager = AppWidgetManager.getInstance(c)
            val ids = manager.getAppWidgetIds(ComponentName(c, CrabWidget::class.java))
            if (ids.isNotEmpty()) render(c, manager, ids)
        }

        private fun clock(t: Long) = SimpleDateFormat("HH:mm", Locale.getDefault()).format(Date(t))

        private fun resetText(iso: String?): String {
            if (iso.isNullOrEmpty()) return ""
            val t = runCatching { java.time.OffsetDateTime.parse(iso).toInstant().toEpochMilli() }.getOrNull()
                ?: return ""
            val left = t - Usage.now()
            if (left <= 0) return "resets any moment"
            val sameDay = left < 20 * 60 * 60 * 1000L
            return if (sameDay) "resets ${clock(t)}"
            else "resets " + SimpleDateFormat("EEE HH:mm", Locale.getDefault()).format(Date(t))
        }

        private fun render(c: Context, manager: AppWidgetManager, ids: IntArray) {
            val good = Store.good(c)
            val latest = Store.latest(c)
            val remaining = Store.flag(c, "remaining")
            val fresh = good != null && Usage.now() - good.optLong("at") < STALE_MS
            val sPct = Usage.sessionPct(good)
            val wPct = Usage.weekPct(good)
            val shown = { p: Double? -> p?.let { if (remaining) 100 - it else it } }
            val label = { p: Double? ->
                if (p == null) "-" else "${shown(p)!!.toInt()}%" + if (remaining) " left" else ""
            }

            val mood = if (fresh) CrabPainter.moodFor(sPct) else CrabPainter.Mood.CALM
            val bitmap = CrabPainter.draw(
                256, mood, shown(sPct), shown(wPct),
                sPct?.let { CrabPainter.levelColor(it) }, fresh,
            )

            val data = good?.optJSONObject("data")
            val status = when (latest.optString("state")) {
                "login" -> "Sign in needed · tap"
                "error" -> if (good != null) "Offline · ${clock(good.optLong("at"))}" else "No data yet · tap"
                "ok" -> "Updated ${clock(latest.optLong("at"))}"
                else -> "Tap to open"
            }

            val views = RemoteViews(c.packageName, R.layout.widget_crab).apply {
                setImageViewBitmap(R.id.crab, bitmap)
                setTextViewText(R.id.session, "5-hour  ${label(sPct)}")
                setTextViewText(R.id.session_sub, resetText(data?.optJSONObject("five_hour")?.optString("resets_at")))
                setTextViewText(R.id.week, "Weekly  ${label(wPct)}")
                setTextViewText(R.id.status, status)
                val open = PendingIntent.getActivity(
                    c, 0, Intent(c, MainActivity::class.java),
                    PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
                )
                setOnClickPendingIntent(R.id.root, open)
            }
            manager.updateAppWidget(ids, views)
        }
    }
}
