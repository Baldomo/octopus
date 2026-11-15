package com.noi.octopus

import android.annotation.SuppressLint
import android.app.Activity
import android.graphics.Color
import android.os.Message
import android.util.Base64
import android.view.View
import android.view.ViewGroup
import android.view.inputmethod.InputMethodManager
import android.webkit.RenderProcessGoneDetail
import android.webkit.WebChromeClient
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.FrameLayout
import androidx.webkit.Navigation
import androidx.webkit.NavigationListener
import androidx.webkit.Page
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Channel
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.ByteArrayInputStream
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicLong
import kotlin.math.roundToInt

@InvokeArg
class Watch {
    lateinit var told: Channel
}

@InvokeArg
class Load {
    lateinit var key: String
    lateinit var url: String
}

@InvokeArg
class Rect {
    var x = 0.0
    var y = 0.0
    var width = 0.0
    var height = 0.0
}

@InvokeArg
class Placed {
    lateinit var key: String
    lateinit var bounds: Rect
    var covered = false
}

@InvokeArg
class Place {
    var page: Placed? = null
}

@InvokeArg
class Went {
    lateinit var key: String
    lateinit var go: String
}

@InvokeArg
class Find {
    lateinit var key: String
    lateinit var text: String
    var backwards = false
}

@InvokeArg
class Forget {
    lateinit var key: String
}

@InvokeArg
class Headed {
    var id = 0L
    var allowed = false
}

@InvokeArg
class Answer {
    var id = 0L
    var status = 0
    lateinit var reason: String
    lateinit var kind: String
    lateinit var headers: Map<String, String>
    lateinit var body: String
}

/** Draws each browser panel's page in a `WebView` of its own beside the app's webview, with no bridge to the app. */
@TauriPlugin
class BrowserPlugin(private val activity: Activity) : Plugin(activity) {
    @SuppressLint("SetJavaScriptEnabled")
    private inner class Shown(val key: String) {
        val view = WebView(activity)
        var loaded = true

        /** Whether the view shows the engine's own error page, which is no page of the browser panel. */
        var failed = false
        var heading: Navigation? = null
        val loading get() = heading != null || !loaded
        var sought: String? = null

        /** The new window the page last asked for, which lives until the page asks for another or goes. */
        var window: WebView? = null

        init {
            view.apply {
                settings.javaScriptEnabled = true
                settings.domStorageEnabled = true
                settings.setSupportMultipleWindows(true)
                settings.builtInZoomControls = true
                settings.displayZoomControls = false
                settings.useWideViewPort = true
                settings.loadWithOverviewMode = true
                webViewClient = client(this@Shown)
                webChromeClient = chrome(this@Shown)
                visibility = View.INVISIBLE
                // A touch takes the focus from a field of the app, whose keyboard would stay up over a page with no field.
                setOnFocusChangeListener { view, focused ->
                    if (focused) {
                        activity.getSystemService(InputMethodManager::class.java).hideSoftInputFromWindow(view.windowToken, 0)
                    }
                }
            }
            WebViewCompat.addNavigationListener(view, listener(this))
            (app.parent as ViewGroup).addView(view, FrameLayout.LayoutParams(0, 0))
        }
    }

    private lateinit var app: WebView
    private var told: Channel? = null
    private val pages = HashMap<String, Shown>()

    /** What each question put to the app waits for, by the id it was asked under. */
    private val waits = ConcurrentHashMap<Long, ArrayBlockingQueue<Any>>()
    private val asked = AtomicLong()

    override fun load(webView: WebView) {
        super.load(webView)
        app = webView
    }

    @Command
    fun watch(invoke: Invoke) {
        told = invoke.parseArgs(Watch::class.java).told
        invoke.resolve(JSObject().put("draws", WebViewFeature.isFeatureSupported(WebViewFeature.NAVIGATION_LISTENER)))
    }

    @Command
    fun load(invoke: Invoke) {
        val load = invoke.parseArgs(Load::class.java)
        activity.runOnUiThread {
            pages.getOrPut(load.key) { Shown(load.key) }.view.loadUrl(load.url)
            invoke.resolve()
        }
    }

    @Command
    fun place(invoke: Invoke) {
        val placed = invoke.parseArgs(Place::class.java).page
        activity.runOnUiThread {
            for ((key, page) in pages) if (key != placed?.key) page.view.visibility = View.INVISIBLE
            if (placed == null) return@runOnUiThread invoke.resolve()
            val page = pages[placed.key] ?: return@runOnUiThread invoke.reject("no browser panel ${placed.key}")
            val density = activity.resources.displayMetrics.density
            val pixels = { css: Double -> (css * density).roundToInt() }
            val bounds = placed.bounds
            page.view.layoutParams = FrameLayout.LayoutParams(pixels(bounds.width), pixels(bounds.height)).apply {
                leftMargin = pixels(bounds.x)
                topMargin = pixels(bounds.y)
            }
            page.view.translationZ = if (placed.covered) UNDER else 0f
            page.view.visibility = View.VISIBLE
            // The camera's scanner leaves the app's webview white when it ends, so each placing clears it.
            app.setBackgroundColor(Color.TRANSPARENT)
            invoke.resolve()
        }
    }

    @Command
    fun go(invoke: Invoke) {
        val went = invoke.parseArgs(Went::class.java)
        val act: (WebView) -> Unit = when (went.go) {
            "back" -> WebView::goBack
            "forward" -> WebView::goForward
            "reload", "reload_from_origin" -> WebView::reload
            else -> return invoke.reject("no way to go ${went.go}")
        }
        onPage(invoke, went.key) { page -> act(page.view) }
    }

    @Command
    fun find(invoke: Invoke) {
        val find = invoke.parseArgs(Find::class.java)
        onPage(invoke, find.key) { page ->
            if (find.text.isEmpty()) {
                page.sought = null
                page.view.clearMatches()
            } else if (find.text == page.sought) {
                page.view.findNext(!find.backwards)
            } else {
                page.sought = find.text
                page.view.findAllAsync(find.text)
            }
        }
    }

    @Command
    fun forget(invoke: Invoke) {
        val key = invoke.parseArgs(Forget::class.java).key
        activity.runOnUiThread {
            pages.remove(key)?.let { page ->
                (page.view.parent as ViewGroup?)?.removeView(page.view)
                page.window?.destroy()
                page.view.destroy()
            }
            invoke.resolve()
        }
    }

    @Command
    fun headed(invoke: Invoke) {
        val headed = invoke.parseArgs(Headed::class.java)
        waits[headed.id]?.offer(headed.allowed)
        invoke.resolve()
    }

    @Command
    fun answer(invoke: Invoke) {
        val answer = invoke.parseArgs(Answer::class.java)
        waits[answer.id]?.offer(answer)
        invoke.resolve()
    }

    private fun onPage(invoke: Invoke, key: String, act: (Shown) -> Unit) {
        activity.runOnUiThread {
            val page = pages[key] ?: return@runOnUiThread invoke.reject("no browser panel $key")
            act(page)
            invoke.resolve()
        }
    }

    private fun tell(told: String, fill: JSObject.() -> Unit) {
        this.told?.send(JSObject().put("told", told).apply(fill))
    }

    private fun changed(page: Shown, change: Any) = tell("changed") {
        put("key", page.key)
        put("change", change)
    }

    private fun seen(page: Shown) = changed(
        page,
        JSObject().put(
            "seen",
            JSObject()
                .put("url", page.view.url.takeUnless { page.failed })
                .put("loading", page.loading)
                .put("back", page.view.canGoBack())
                .put("forward", page.view.canGoForward()),
        ),
    )

    /** Asks the app, and waits off the main thread for what it answers; null once it has taken too long. */
    private fun ask(told: String, fill: JSObject.() -> Unit): Any? {
        val id = asked.incrementAndGet()
        val answer = ArrayBlockingQueue<Any>(1)
        waits[id] = answer
        tell(told) {
            put("id", id)
            fill()
        }
        return answer.poll(ASK_SECONDS, TimeUnit.SECONDS).also { waits.remove(id) }
    }

    private fun listener(page: Shown) = object : NavigationListener {
        override fun onNavigationStarted(navigation: Navigation) {
            if (!navigation.isSameDocument) page.heading = navigation
            seen(page)
        }

        override fun onNavigationCompleted(navigation: Navigation) {
            val arrived = page.heading === navigation
            if (arrived) page.heading = null
            if (navigation.didCommitErrorPage()) {
                page.loaded = true
                page.failed = true
                changed(page, JSObject().put("failed", JSObject().put("url", navigation.url).put("error", why(navigation))))
            } else if (navigation.didCommit()) {
                page.failed = false
                if (arrived) page.loaded = false
            }
            seen(page)
        }

        override fun onPageLoadEvent(loaded: Page) {
            page.loaded = true
            seen(page)
        }
    }

    private fun why(navigation: Navigation): String {
        val described = WebViewFeature.isFeatureSupported(WebViewFeature.NAVIGATION_GET_WEB_RESOURCE_ERROR)
        val error = if (described) navigation.webResourceError?.description?.toString() else null
        return error ?: "the page did not load"
    }

    private fun client(page: Shown) = object : WebViewClient() {
        override fun shouldInterceptRequest(view: WebView, request: WebResourceRequest): WebResourceResponse? {
            val host = request.url.host ?: return null
            if (host.startsWith(FILES) && host.endsWith(LOCAL)) return file(request)
            if (!request.isForMainFrame) return null
            val allowed = ask("heading") {
                put("key", page.key)
                put("url", request.url.toString())
            }
            if (allowed == true) return null
            stop(view)
            return refusal()
        }

        /** Android ended the page's renderer to reclaim its memory, and a view without one can only go. */
        override fun onRenderProcessGone(view: WebView, detail: RenderProcessGoneDetail): Boolean {
            changed(page, "ended")
            return true
        }
    }

    /** Ends the navigation the app refused before its answer could commit as a page; waits off the main thread. */
    private fun stop(view: WebView) {
        val stopped = CountDownLatch(1)
        activity.runOnUiThread {
            view.stopLoading()
            stopped.countDown()
        }
        stopped.await(ASK_SECONDS, TimeUnit.SECONDS)
    }

    private fun file(request: WebResourceRequest): WebResourceResponse {
        val answer = ask("asked") {
            put("url", request.url.toString())
            put("method", request.method)
            put("range", request.requestHeaders["Range"])
        } as? Answer ?: return refusal()
        val charset = if (answer.kind.contains("charset=utf-8")) "utf-8" else null
        return WebResourceResponse(
            answer.kind.substringBefore(';').trim(),
            charset,
            answer.status,
            answer.reason,
            answer.headers,
            ByteArrayInputStream(Base64.decode(answer.body, Base64.DEFAULT)),
        )
    }

    private fun refusal() =
        WebResourceResponse("text/plain", "utf-8", 502, "Bad Gateway", emptyMap(), ByteArrayInputStream(ByteArray(0)))

    private fun chrome(page: Shown) = object : WebChromeClient() {
        /** A page with no title reports its address as one, which is none, and the engine's error page its own. */
        override fun onReceivedTitle(view: WebView, title: String?) {
            if (page.failed) return
            val bare = { text: String -> text.substringAfter("://").trimEnd('/') }
            changed(page, JSObject().put("titled", title?.takeUnless { bare(it) == bare(view.url.orEmpty()) } ?: ""))
        }

        /** A new window's first navigation names the page it opens, which opens as a tab in place of the window. */
        override fun onCreateWindow(view: WebView, isDialog: Boolean, isUserGesture: Boolean, resultMsg: Message): Boolean {
            page.window?.destroy()
            val window = WebView(activity)
            page.window = window
            window.webViewClient = object : WebViewClient() {
                var named = false

                override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
                    if (!named) {
                        named = true
                        changed(page, JSObject().put("opened", JSObject().put("url", request.url.toString()).put("select", true)))
                    }
                    return true
                }
            }
            (resultMsg.obj as WebView.WebViewTransport).webView = window
            resultMsg.sendToTarget()
            return true
        }
    }

    private companion object {
        const val FILES = "octopus-file-"
        const val LOCAL = ".localhost"
        const val UNDER = -1f
        const val ASK_SECONDS = 30L
    }
}
