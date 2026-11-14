package local.crabwidget.android

import android.annotation.SuppressLint
import android.content.Context
import android.net.Uri
import android.webkit.WebView
import android.webkit.WebViewClient
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import org.json.JSONObject
import org.json.JSONTokener
import kotlin.coroutines.resume

/**
 * Reads Claude usage through a hidden WebView on claude.ai that carries the user's own login
 * (the cookies of this app's WebView profile). The app never sees a password or token.
 *
 * Like the desktop widget, the claude.ai page gets no bridge into the app: a script runs in it,
 * leaves its JSON answer in a page variable, and the app reads that variable back.
 */
class ClaudeProbe(context: Context) {
    private val app = context.applicationContext
    private var web: WebView? = null
    private var pageLoaded: CompletableDeferred<Unit>? = null
    private val lock = Mutex()

    /** {ok:true, usage:{...}} or {ok:false, status, stage}. */
    suspend fun usage(): JSONObject = run(PROBE_JS)
        ?: JSONObject().put("ok", false).put("status", -2).put("stage", "no-answer")

    /** {ok:true, chats:[...]} or {ok:false, status}, or null when there was no answer. */
    suspend fun chats(): JSONObject? = run(CHATS_JS)

    /** Lets go of the WebView. Safe to call from any thread. */
    fun destroy() {
        val w = web ?: return
        web = null
        w.post { w.destroy() }
    }

    private suspend fun run(js: String): JSONObject? = lock.withLock {
        withContext(Dispatchers.Main) {
            val w = ready() ?: return@withContext null
            runIn(w, js)
        }
    }

    @SuppressLint("SetJavaScriptEnabled")
    private fun webView(): WebView = web ?: WebView(app).also { w ->
        w.settings.javaScriptEnabled = true
        w.settings.domStorageEnabled = true
        w.webViewClient = object : WebViewClient() {
            override fun onPageFinished(view: WebView, url: String) {
                pageLoaded?.complete(Unit)
            }
        }
        web = w
    }

    /** A WebView sitting on a claude.ai page, loading one first if needed. */
    private suspend fun ready(): WebView? {
        val w = webView()
        if (!onClaude(w.url)) {
            val loaded = CompletableDeferred<Unit>()
            pageLoaded = loaded
            w.loadUrl(IDLE_URL)
            withTimeoutOrNull(20_000) { loaded.await() }
        }
        return w.takeIf { onClaude(it.url) }
    }

    companion object {
        private const val HOST = "claude.ai"
        /** A JSON endpoint on the same origin: cheap to load as the hidden page. */
        const val IDLE_URL = "https://claude.ai/api/organizations"
        const val LOGIN_URL = "https://claude.ai/login"
        const val USAGE_PAGE = "https://claude.ai/settings/usage"

        fun onClaude(url: String?): Boolean =
            url != null && Uri.parse(url).let { it.scheme == "https" && it.host == HOST }

        /** Runs [js] in a claude.ai page and waits for the answer it leaves in window.__crab. */
        suspend fun runIn(w: WebView, js: String): JSONObject? {
            w.evaluateJavascript("window.__crab = null;$js", null)
            repeat(30) {
                delay(500)
                val raw = eval(w, "window.__crab || ''")
                val text = runCatching { JSONTokener(raw).nextValue() }.getOrNull() as? String
                if (!text.isNullOrEmpty()) return runCatching { JSONObject(text) }.getOrNull()
            }
            return null
        }

        private suspend fun eval(w: WebView, js: String): String =
            suspendCancellableCoroutine { cont -> w.evaluateJavascript(js) { cont.resume(it ?: "null") } }

        /** Picks the organization with a Claude plan, as the desktop widget does. */
        private const val ORG_JS = """
          const get = (path) => fetch(path, { credentials: 'include', headers: { accept: 'application/json' } });
          const send = (payload) => { payload.t = Date.now(); window.__crab = JSON.stringify(payload); };
          const pickOrg = async () => {
            if (window.__crabOrg) return { uuid: window.__crabOrg };
            const r = await get('/api/organizations');
            if (!r.ok) return { status: r.status };
            const orgs = await r.json();
            const caps = (o) => o.capabilities || [];
            const org = orgs.find((o) => caps(o).some((c) => /claude_(pro|max)/.test(c)))
              || orgs.find((o) => caps(o).includes('chat'))
              || orgs[0];
            if (!org) return { status: 0 };
            window.__crabOrg = org.uuid;
            return { uuid: org.uuid };
          };
        """

        /** Reads the usage limits: {ok:true, usage} or {ok:false, status, stage}. */
        const val PROBE_JS = """
        (async () => {
          $ORG_JS
          try {
            window.__crabOrg = null;
            const org = await pickOrg();
            if (!org.uuid) return send({ ok: false, status: org.status, stage: 'organizations' });
            const u = await get('/api/organizations/' + org.uuid + '/usage');
            if (!u.ok) return send({ ok: false, status: u.status, stage: 'usage' });
            send({ ok: true, usage: await u.json() });
          } catch (e) {
            send({ ok: false, status: -1, stage: String(e) });
          }
        })();
        """

        /** The newest chats: id, title and status fields only, never message text. */
        private const val CHATS_JS = """
        (async () => {
          $ORG_JS
          try {
            const org = await pickOrg();
            if (!org.uuid) return send({ ok: false, status: org.status });
            const c = await get('/api/organizations/' + org.uuid + '/chat_conversations?limit=8');
            if (!c.ok) return send({ ok: false, status: c.status });
            const list = await c.json();
            send({
              ok: true,
              chats: list.map((x) => ({
                id: x.uuid,
                name: (x.name || '').slice(0, 90),
                live: x.live_status ?? null,
                needs: !!x.needs_input,
                u: Date.parse(x.updated_at) || null,
                s: x.last_settled_at ? Date.parse(x.last_settled_at) : null,
              })),
            });
          } catch (e) {
            send({ ok: false, status: -1 });
          }
        })();
        """
    }
}
