package local.crabwidget.android

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.RectF
import android.graphics.Typeface

/**
 * Draws the crab and its rings into a bitmap for the home-screen widget, which cannot run the
 * app's animated page. Same pixel shapes and coordinates as the SVG crab, one still pose per mood.
 */
object CrabPainter {
    enum class Mood { CALM, WARM, PANIC, SLEEP }

    private val SHELL = Color.parseColor("#d97757")
    private val SHELL_PANIC = Color.parseColor("#e0483a")
    private val SHELL_SLEEP = Color.parseColor("#8f6a5d")
    private val INK = Color.parseColor("#1d1b1a")
    private val SWEAT = Color.parseColor("#7cc4ff")
    private val OFF = Color.parseColor("#8a8a92")
    private val CLAUDE = Color.parseColor("#e8845c")

    fun moodFor(pct: Double?): Mood = when {
        pct == null -> Mood.CALM
        pct >= 100 -> Mood.SLEEP
        pct >= 80 -> Mood.PANIC
        pct >= 50 -> Mood.WARM
        else -> Mood.CALM
    }

    fun levelColor(pct: Double): Int = when {
        pct >= 95 -> Color.parseColor("#e0483a")
        pct >= 75 -> Color.parseColor("#ff9a1f")
        else -> CLAUDE
    }

    /** [session] and [week] are what the rings show (used or left), 0-100, or null. */
    fun draw(size: Int, mood: Mood, session: Double?, week: Double?, sessionColor: Int?, fresh: Boolean): Bitmap {
        val bmp = Bitmap.createBitmap(size, size, Bitmap.Config.ARGB_8888)
        val c = Canvas(bmp)
        c.scale(size / 150f, size / 150f)
        val fill = Paint(Paint.ANTI_ALIAS_FLAG)
        val stroke = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            style = Paint.Style.STROKE
            strokeWidth = 6f
            strokeCap = Paint.Cap.ROUND
        }

        // backdrop and ring tracks
        fill.color = Color.argb(225, 20, 19, 22)
        c.drawCircle(75f, 75f, 74f, fill)
        stroke.color = Color.argb(30, 255, 255, 255)
        c.drawCircle(75f, 75f, 67f, stroke)
        c.drawCircle(75f, 75f, 59f, stroke)

        fun arc(r: Float, pct: Double?, color: Int) {
            if (pct == null || pct <= 0) return
            stroke.color = color
            val box = RectF(75 - r, 75 - r, 75 + r, 75 + r)
            c.drawArc(box, -90f, (360 * pct.coerceAtMost(100.0) / 100).toFloat(), false, stroke)
        }
        arc(67f, session, if (fresh) sessionColor ?: CLAUDE else OFF)
        arc(59f, week, if (fresh) Color.argb(160, 232, 132, 92) else OFF)

        // the crab, in its own SVG coordinates
        c.save()
        c.translate(32.5f + 4f, 40f)
        c.scale(0.73f, 0.73f)
        c.translate(8f, 22f)
        fill.isAntiAlias = false
        val shell = when (mood) {
            Mood.PANIC -> SHELL_PANIC
            Mood.SLEEP -> SHELL_SLEEP
            else -> SHELL
        }
        fun rect(x: Float, y: Float, w: Float, h: Float, color: Int) {
            fill.color = color
            c.drawRect(x, y, x + w, y + h, fill)
        }

        val my = if (mood == Mood.SLEEP) 9f else 0f
        val legLift = if (mood == Mood.SLEEP) -10f else 0f
        for (x in listOf(16f, 28f, 56f, 68f)) rect(x, 36 + my + legLift, 6f, 16f, shell)
        val armL = if (mood == Mood.PANIC) -11f else 0f
        rect(0f, 14 + my + armL, 12f, 12f, shell)
        rect(78f, 14 + my, 12f, 12f, shell)
        rect(10f, my, 70f, 38f, shell)
        val eyeH = when (mood) {
            Mood.PANIC -> 16f
            Mood.SLEEP -> 2f
            else -> 13f
        }
        val eyeY = 10 + (13 - eyeH) / 2 + my + if (mood == Mood.SLEEP) 3f else 0f
        rect(24f, eyeY, 6f, eyeH, INK)
        rect(60f, eyeY, 6f, eyeH, INK)

        if (mood == Mood.WARM || mood == Mood.PANIC) rect(83f, 8f, 4f, 6f, SWEAT)
        if (mood == Mood.PANIC) rect(3f, 14f, 4f, 6f, SWEAT)
        if (mood == Mood.SLEEP) {
            val text = Paint(Paint.ANTI_ALIAS_FLAG).apply {
                color = Color.parseColor("#f2efe9")
                textSize = 17f
                typeface = Typeface.DEFAULT_BOLD
            }
            c.drawText("z", 80f, -2f, text)
            text.textSize = 12f
            c.drawText("z", 90f, -14f, text)
        }
        c.restore()
        return bmp
    }
}
