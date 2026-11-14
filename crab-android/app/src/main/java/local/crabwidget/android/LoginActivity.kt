package local.crabwidget.android

import android.annotation.SuppressLint
import android.app.Activity
import android.graphics.Color
import android.os.Bundle
import android.view.Gravity
import android.view.ViewGroup
import android.webkit.CookieManager
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * claude.ai's own sign-in page in a WebView. The session stays in this app's WebView cookies.
 * Closes itself as soon as usage can be read.
 */
class LoginActivity : Activity() {
    private lateinit var web: WebView
    private lateinit var status: TextView
    private val scope = MainScope()
    private var probing = false

    @SuppressLint("SetJavaScriptEnabled")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val dp = resources.displayMetrics.density
        status = TextView(this).apply {
            text = "Sign in with your email. Google sign-in does not work inside apps."
            textSize = 13f
            setPadding((16 * dp).toInt(), (10 * dp).toInt(), (8 * dp).toInt(), (10 * dp).toInt())
        }
        val close = Button(this).apply {
            text = "Close"
            setOnClickListener { finish() }
        }
        val bar = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            addView(status, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
            addView(close)
        }
        web = WebView(this).apply {
            settings.javaScriptEnabled = true
            settings.domStorageEnabled = true
            webViewClient = object : WebViewClient() {
                override fun onPageFinished(view: WebView, url: String) {
                    // Past the login page on claude.ai: see whether usage can be read yet.
                    if (ClaudeProbe.onClaude(url) && !url.contains("/login")) tryFinish()
                }
            }
        }
        CookieManager.getInstance().setAcceptCookie(true)
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(getColor(R.color.page_bg))
            fitsSystemWindows = true
            addView(bar, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT))
            addView(web, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f))
        }
        setContentView(root)
        web.loadUrl(ClaudeProbe.LOGIN_URL)
        // claude.ai moves on after sign-in without a full page load, so look every few seconds too.
        scope.launch {
            while (true) {
                delay(4_000)
                val url = web.url
                if (ClaudeProbe.onClaude(url) && url?.contains("/login") == false) tryFinish()
            }
        }
    }

    private fun tryFinish() {
        if (probing) return
        probing = true
        scope.launch {
            try {
                val answer = ClaudeProbe.runIn(web, ClaudeProbe.PROBE_JS)
                if (answer != null && answer.optBoolean("ok") && Usage.record(this@LoginActivity, answer)) {
                    CookieManager.getInstance().flush()
                    status.setTextColor(Color.parseColor("#2e9e5b"))
                    status.text = "Signed in. Back to the crab..."
                    finish()
                }
            } finally {
                probing = false
            }
        }
    }

    override fun onDestroy() {
        scope.cancel()
        web.destroy()
        super.onDestroy()
    }
}
