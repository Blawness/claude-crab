package local.crabwidget.android

import android.Manifest
import android.annotation.SuppressLint
import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.view.HapticFeedbackConstants
import android.webkit.JavascriptInterface
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient
import kotlinx.coroutines.Job
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import org.json.JSONObject

/**
 * The crab screen. The page is the app's own (assets/index.html) and is the only page that
 * gets the "Crab" bridge; claude.ai is only ever loaded in [ClaudeProbe] and [LoginActivity].
 */
class MainActivity : Activity() {
    private lateinit var ui: WebView
    private val scope = MainScope()
    private var probe: ClaudeProbe? = null
    private var loop: Job? = null
    private var checking = false
    private var lastUsageCheck = 0L
    private var lastChatCheck = 0L
    /** idle, checking, downloading, installing or error. */
    private var updatePhase = "idle"
    private var updateProgress = -1
    private var updateError = ""

    @SuppressLint("SetJavaScriptEnabled")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        ui = WebView(this)
        ui.setBackgroundColor(getColor(R.color.page_bg))
        ui.settings.javaScriptEnabled = true
        ui.addJavascriptInterface(Bridge(), "Crab")
        ui.webViewClient = object : WebViewClient() {
            // Every link leaves the app page: claude.ai chat links open in the Claude app or browser.
            override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
                openExternal(request.url.toString())
                return true
            }
        }
        // Edge to edge: the page gets the system bar sizes as padding.
        ui.setOnApplyWindowInsetsListener { v, insets ->
            @Suppress("DEPRECATION")
            v.setPadding(insets.systemWindowInsetLeft, insets.systemWindowInsetTop, insets.systemWindowInsetRight, insets.systemWindowInsetBottom)
            insets
        }
        window.setDecorFitsSystemWindowsCompat()
        setContentView(ui)
        ui.loadUrl("file:///android_asset/index.html")

        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != android.content.pm.PackageManager.PERMISSION_GRANTED) {
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1)
        }
    }

    private fun android.view.Window.setDecorFitsSystemWindowsCompat() {
        if (Build.VERSION.SDK_INT >= 30) setDecorFitsSystemWindows(false)
        else {
            @Suppress("DEPRECATION")
            decorView.systemUiVisibility = android.view.View.SYSTEM_UI_FLAG_LAYOUT_STABLE or
                android.view.View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN or
                android.view.View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
        }
    }

    override fun onResume() {
        super.onResume()
        current = this
        ui.onResume()
        // Back from the install screen without installing, or from the unknown-apps setting.
        if (updatePhase == "installing") updatePhase = "idle"
        push()
        scope.launch {
            runCatching { Updater.check(this@MainActivity) }
            push()
        }
        loop = scope.launch {
            while (isActive) {
                val now = Usage.now()
                val usageDue = now - lastUsageCheck >= Store.interval(this@MainActivity) * 1000
                val chatsDue = Store.flag(this@MainActivity, "chats") && now - lastChatCheck >= CHAT_MS
                if (usageDue) checkUsage() else if (chatsDue) checkChats()
                pushPhone()
                delay(5_000)
            }
        }
    }

    override fun onPause() {
        super.onPause()
        loop?.cancel()
        ui.onPause() // hides the page, so the crab's ticker stops
        // The hidden claude.ai page only lives while the crab is on screen.
        probe?.destroy()
        probe = null
    }

    override fun onDestroy() {
        if (current === this) current = null
        scope.cancel()
        ui.destroy()
        super.onDestroy()
    }

    private fun probe() = probe ?: ClaudeProbe(this).also { probe = it }

    private suspend fun checkUsage() {
        if (checking) return
        checking = true
        push(checking = true)
        try {
            lastUsageCheck = Usage.now()
            Usage.record(this, probe().usage())
            if (Store.flag(this, "chats")) checkChats(pushAfter = false)
        } finally {
            checking = false
            push()
        }
    }

    private suspend fun checkChats(pushAfter: Boolean = true) {
        lastChatCheck = Usage.now()
        val answer = probe().chats()
        // Signed out or offline: back off for a minute.
        if (answer == null || !answer.optBoolean("ok")) lastChatCheck += 30_000
        Usage.recordChats(this, answer)
        if (pushAfter) push()
    }

    private fun refreshNow() {
        lastUsageCheck = 0
        scope.launch { checkUsage() }
    }

    private fun state(checking: Boolean = this.checking): JSONObject = JSONObject()
        .put("latest", Store.latest(this))
        .put("good", Store.good(this) ?: JSONObject.NULL)
        .put("history", Store.history(this))
        .put("chats", if (Store.flag(this, "chats")) Store.chats(this) ?: JSONObject.NULL else JSONObject.NULL)
        .put("settings", Store.settings(this))
        .put("phone", Usage.phone(this))
        .put("checking", checking)
        .put("update", updateState())

    private fun updateState(): JSONObject {
        val newer = Updater.pending(this)
        return JSONObject()
            .put("current", "${Updater.currentName(this)} (build ${Updater.currentCode(this)})")
            .put("latest", newer?.let { JSONObject().put("code", it.code).put("name", it.name) } ?: JSONObject.NULL)
            .put("phase", updatePhase)
            .put("progress", updateProgress)
            .put("error", updateError)
            .put("checkedAt", Store.lastUpdateCheck(this))
    }

    private fun checkUpdateNow() {
        if (updatePhase == "checking" || updatePhase == "downloading") return
        updatePhase = "checking"
        updateError = ""
        push()
        scope.launch {
            val ok = runCatching { Updater.check(this@MainActivity, force = true) }.isSuccess
            updatePhase = if (ok) "idle" else "error"
            if (!ok) updateError = "Could not reach GitHub."
            push()
        }
    }

    private fun installUpdate() {
        val release = Updater.pending(this) ?: return
        if (updatePhase == "downloading" || updatePhase == "installing") return
        if (!Updater.canInstall(this)) {
            // One-time: Android asks the user to allow installs from this app.
            updateError = "Allow \"Install unknown apps\" for Claude Crab, then tap Update again."
            updatePhase = "error"
            push()
            runCatching {
                startActivity(Intent(android.provider.Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:$packageName")))
            }
            return
        }
        updatePhase = "downloading"
        updateProgress = 0
        updateError = ""
        push()
        scope.launch {
            try {
                val apk = Updater.download(this@MainActivity, release) { pct ->
                    updateProgress = pct
                    push()
                }
                updatePhase = "installing"
                push()
                Updater.install(this@MainActivity, apk)
            } catch (e: Exception) {
                updatePhase = "error"
                updateError = e.message ?: "Update failed."
                push()
            }
        }
    }

    private fun onInstallFailed(why: String) {
        updatePhase = "error"
        updateError = "Install failed: $why"
        push()
    }

    private fun push(checking: Boolean = this.checking) {
        ui.evaluateJavascript("window.crabUpdate && crabUpdate(${state(checking)})", null)
    }

    private fun pushPhone() {
        ui.evaluateJavascript("window.crabPhone && crabPhone(${Usage.phone(this)})", null)
    }

    private fun openExternal(url: String) {
        val uri = Uri.parse(url)
        if (uri.scheme != "https") return
        runCatching { startActivity(Intent(Intent.ACTION_VIEW, uri)) }
    }

    /** What the crab page may ask for. Runs on a WebView thread, so everything hops to main. */
    inner class Bridge {
        @JavascriptInterface
        fun state(): String = this@MainActivity.state().toString()

        @JavascriptInterface
        fun refresh() = runOnUiThread { refreshNow() }

        @JavascriptInterface
        fun login() = runOnUiThread {
            startActivity(Intent(this@MainActivity, LoginActivity::class.java))
        }

        @JavascriptInterface
        fun openUsagePage() = runOnUiThread { openExternal(ClaudeProbe.USAGE_PAGE) }

        @JavascriptInterface
        fun openChat(id: String) = runOnUiThread {
            if (id.matches(Regex("[A-Za-z0-9-]{1,64}"))) openExternal("https://claude.ai/chat/$id")
        }

        @JavascriptInterface
        fun haptic() = runOnUiThread { ui.performHapticFeedback(HapticFeedbackConstants.VIRTUAL_KEY) }

        @JavascriptInterface
        fun setFlag(key: String, on: Boolean) = runOnUiThread {
            if (key !in FLAGS) return@runOnUiThread
            Store.setFlag(this@MainActivity, key, on)
            when (key) {
                "background" -> CrabWorker.sync(this@MainActivity)
                "remaining" -> CrabWidget.refresh(this@MainActivity)
                "chats" -> if (on) {
                    lastChatCheck = 0
                } else Store.saveChats(this@MainActivity, null)
            }
            push()
        }

        @JavascriptInterface
        fun checkUpdate() = runOnUiThread { checkUpdateNow() }

        @JavascriptInterface
        fun installUpdate() = runOnUiThread { this@MainActivity.installUpdate() }

        @JavascriptInterface
        fun setInterval(secs: Int) = runOnUiThread {
            Store.setInterval(this@MainActivity, secs.toLong())
            push()
        }
    }

    companion object {
        private var current: MainActivity? = null

        /** From [InstallReceiver]: shows why Android refused the update. */
        fun installFailed(why: String) {
            current?.let { a -> a.runOnUiThread { a.onInstallFailed(why) } }
        }

        private const val CHAT_MS = 30_000L
        private val FLAGS = setOf("remaining", "smooth", "chats", "background", "notify")
    }
}
