package com.noi.octopus

import android.annotation.SuppressLint
import android.app.Activity
import android.app.DownloadManager
import android.content.BroadcastReceiver
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.graphics.Color
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.os.Handler
import android.os.Looper
import android.os.Message
import android.provider.MediaStore
import android.util.Base64
import android.view.View
import android.view.ViewGroup
import android.view.inputmethod.InputMethodManager
import android.webkit.CookieManager
import android.webkit.RenderProcessGoneDetail
import android.webkit.MimeTypeMap
import android.webkit.URLUtil
import android.webkit.WebChromeClient
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.FrameLayout
import androidx.core.content.ContextCompat
import androidx.webkit.Navigation
import androidx.webkit.NavigationListener
import androidx.webkit.Page
import androidx.webkit.WebMessageCompat
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
import java.io.File
import java.util.UUID
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicLong
import kotlin.concurrent.thread
import kotlin.math.roundToInt
import org.json.JSONObject

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
                setOnLongClickListener { held(this@Shown) }
                setDownloadListener { url, agent, disposition, kind, _ -> download(this@Shown, url, agent, disposition, kind) }
                visibility = View.INVISIBLE
                // A touch takes the focus from a field of the app, whose keyboard would stay up over a page with no field.
                setOnFocusChangeListener { view, focused ->
                    if (focused) {
                        activity.getSystemService(InputMethodManager::class.java).hideSoftInputFromWindow(view.windowToken, 0)
                    }
                }
            }
            WebViewCompat.addNavigationListener(view, listener(this))
            WebViewCompat.addWebMessageListener(view, READER, setOf("*")) { view, message, _, isMainFrame, _ ->
                if (isMainFrame) read(view, message)
            }
            (app.parent as ViewGroup).addView(view, FrameLayout.LayoutParams(0, 0))
        }
    }

    private lateinit var app: WebView
    private var told: Channel? = null
    private val pages = HashMap<String, Shown>()

    /** The name each download under way saves as, and the key of the page that started it, by its id. */
    private val downloading = HashMap<Long, Pair<String, String>>()

    /** A download a page reads itself: the page's key, the file's name and its type. */
    private class Reading(val key: String, val name: String, val kind: String?)

    /** Each download a page reads itself, by the token its reply carries. */
    private val reading = HashMap<String, Reading>()

    /** Per page's view, the download whose bytes its page posts next. */
    private val arriving = HashMap<WebView, Reading>()

    /** What each question put to the app waits for, by the id it was asked under. */
    private val waits = ConcurrentHashMap<Long, ArrayBlockingQueue<Any>>()
    private val asked = AtomicLong()

    override fun load(webView: WebView) {
        super.load(webView)
        app = webView
        val finished = object : BroadcastReceiver() {
            override fun onReceive(context: Context, intent: Intent) =
                downloaded(intent.getLongExtra(DownloadManager.EXTRA_DOWNLOAD_ID, -1))
        }
        val filter = IntentFilter(DownloadManager.ACTION_DOWNLOAD_COMPLETE)
        ContextCompat.registerReceiver(activity, finished, filter, ContextCompat.RECEIVER_EXPORTED)
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
            // A page under the app's sheet or menu leaves the focus to the app's webview, whose clipboard writes need it.
            page.view.isFocusable = !placed.covered
            if (placed.covered) app.requestFocus()
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

    private fun changed(page: Shown, change: Any) = changed(page.key, change)

    private fun changed(key: String, change: Any) = tell("changed") {
        put("key", key)
        put("change", change)
    }

    /** A link a finger holds shows the app's own menu, as the engine shows none; anything else keeps the engine's. */
    private fun held(page: Shown): Boolean {
        val hit = page.view.hitTestResult
        val link = { url: String -> changed(page, JSObject().put("held", JSObject().put("link", url))) }
        when (hit.type) {
            WebView.HitTestResult.SRC_ANCHOR_TYPE -> link(hit.extra ?: return false)
            WebView.HitTestResult.SRC_IMAGE_ANCHOR_TYPE -> {
                val named = Handler(Looper.getMainLooper()) { message ->
                    message.data.getString("url")?.let(link)
                    true
                }
                page.view.requestFocusNodeHref(named.obtainMessage())
            }
            else -> return false
        }
        return true
    }

    /**
     * The system's download manager saves a web download, with the page's cookies, to Downloads, or on Android 9 and
     * earlier, which let an app write there only with a permission it asks for, to the app's own Downloads folder. A
     * `blob:` or `data:` download, or a file page's, which it cannot fetch, the page reads itself.
     */
    private fun download(page: Shown, url: String, agent: String, disposition: String?, kind: String?) {
        val uri = Uri.parse(url)
        // A file page's file keeps its own name, which the engine would give the extension of the type it is served as.
        val name = uri.lastPathSegment?.takeIf { served(uri) } ?: URLUtil.guessFileName(url, disposition, kind)
        if (uri.scheme !in setOf("http", "https") || served(uri)) return readInPage(page, url, Reading(page.key, name, kind))
        val started = runCatching {
            val request = DownloadManager.Request(uri)
                .setMimeType(kind)
                .addRequestHeader("User-Agent", agent)
                .setNotificationVisibility(DownloadManager.Request.VISIBILITY_VISIBLE_NOTIFY_COMPLETED)
            CookieManager.getInstance().getCookie(url)?.let { request.addRequestHeader("Cookie", it) }
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                request.setDestinationInExternalPublicDir(Environment.DIRECTORY_DOWNLOADS, name)
            } else {
                request.setDestinationInExternalFilesDir(activity, Environment.DIRECTORY_DOWNLOADS, name)
            }
            manager().enqueue(request)
        }
        started.onSuccess { id -> downloading[id] = name to page.key }
        started.onFailure { ended(page.key, name, false) }
    }

    /**
     * A download the download manager says ended, named by the file it saved, which it may have renamed; any app may
     * send the broadcast, so a download still under way goes on, and one the user cancelled, whose row is gone, failed.
     */
    private fun downloaded(id: Long) {
        val (name, key) = downloading[id] ?: return
        manager().query(DownloadManager.Query().setFilterById(id)).use { row ->
            val status = if (row.moveToFirst()) row.getInt(row.getColumnIndexOrThrow(DownloadManager.COLUMN_STATUS)) else null
            if (status == DownloadManager.STATUS_PENDING || status == DownloadManager.STATUS_RUNNING || status == DownloadManager.STATUS_PAUSED) return
            downloading.remove(id)
            val saved = status?.let { row.getString(row.getColumnIndexOrThrow(DownloadManager.COLUMN_LOCAL_URI)) }
            ended(key, saved?.let { Uri.parse(it).lastPathSegment } ?: name, status == DownloadManager.STATUS_SUCCESSFUL)
        }
    }

    /**
     * Has the page fetch `url` and post back, under a token only this download knows, the name a link's `download`
     * attribute gives it, which the engine leaves out of the download it reports, then the bytes it read.
     */
    private fun readInPage(page: Shown, url: String, download: Reading) {
        val token = UUID.randomUUID().toString()
        reading[token] = download
        val script = """(async (url, token) => {
            const header = (name, ok) => $READER.postMessage(JSON.stringify({ token, name, ok }));
            try {
                const name = [...document.querySelectorAll("a[download]")].find((link) => link.href === url)?.download;
                const bytes = await (await fetch(url)).arrayBuffer();
                header(name ?? "", true);
                $READER.postMessage(bytes);
            } catch {
                header("", false);
            }
        })(${JSONObject.quote(url)}, "$token")"""
        page.view.evaluateJavascript(script, null)
    }

    /**
     * Hears what a page posts for a download it reads: a header naming the download by its token, then its bytes,
     * which `save` writes off the main thread. Any page can post, so anything else is dropped.
     */
    private fun read(view: WebView, message: WebMessageCompat) {
        if (message.type == WebMessageCompat.TYPE_ARRAY_BUFFER) {
            val download = arriving.remove(view) ?: return
            val bytes = message.arrayBuffer
            thread {
                val saved = runCatching { save(download, bytes) }
                ended(download.key, saved.getOrDefault(download.name), saved.isSuccess)
            }
            return
        }
        val header = runCatching { JSONObject(checkNotNull(message.data)) }.getOrNull() ?: return
        val asked = reading.remove(header.optString("token")) ?: return
        val name = File(header.optString("name")).name.takeIf { it.isNotBlank() } ?: asked.name
        val download = Reading(asked.key, name, asked.kind)
        if (header.optBoolean("ok")) arriving[view] = download else ended(download.key, name, false)
    }

    /**
     * Writes bytes where the download manager would, under a name no file there has, and gives the name it took:
     * through the media store to Downloads, or on Android 9 and earlier to the app's own Downloads folder.
     */
    private fun save(download: Reading, bytes: ByteArray): String {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) {
            val dot = download.name.lastIndexOf('.').takeIf { it > 0 } ?: download.name.length
            val (stem, extension) = download.name.substring(0, dot) to download.name.substring(dot)
            val file = generateSequence(0) { it + 1 }
                .map { nth -> File(ownDownloads(), if (nth == 0) download.name else "$stem-$nth$extension") }
                .first { it.createNewFile() }
            file.writeBytes(bytes)
            return file.name
        }
        val resolver = activity.contentResolver
        // The media store gives a file the extension of its type, so the type follows the name.
        val type = MimeTypeMap.getSingleton().getMimeTypeFromExtension(download.name.substringAfterLast('.', ""))
        val values = ContentValues().apply {
            put(MediaStore.Downloads.DISPLAY_NAME, download.name)
            put(MediaStore.Downloads.MIME_TYPE, type ?: download.kind ?: "application/octet-stream")
            put(MediaStore.Downloads.IS_PENDING, 1)
        }
        val uri = checkNotNull(resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)) { "no room in Downloads" }
        runCatching { checkNotNull(resolver.openOutputStream(uri)) { "Downloads took no bytes" }.use { it.write(bytes) } }
            .onFailure { resolver.delete(uri, null, null) }
            .getOrThrow()
        resolver.update(uri, ContentValues().apply { put(MediaStore.Downloads.IS_PENDING, 0) }, null, null)
        return checkNotNull(resolver.query(uri, arrayOf(MediaStore.Downloads.DISPLAY_NAME), null, null, null)).use {
            it.moveToFirst()
            it.getString(0)
        }
    }

    private fun ownDownloads() = checkNotNull(activity.getExternalFilesDir(Environment.DIRECTORY_DOWNLOADS))

    /** Whether a URL is a file page's, which the app serves and nothing outside the page reaches. */
    private fun served(uri: Uri) = uri.host?.let { it.startsWith(FILES) && it.endsWith(LOCAL) } == true

    /** Where a download lands, as the user finds it: Downloads, or on Android 9 and earlier the app's own folder. */
    private val folderLabel by lazy {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            "Downloads"
        } else {
            @Suppress("DEPRECATION")
            ownDownloads().relativeTo(Environment.getExternalStorageDirectory()).path
        }
    }

    private fun ended(key: String, file: String, success: Boolean) = changed(
        key,
        JSObject().put("downloaded", JSObject().put("file", file).put("folder", folderLabel).put("success", success)),
    )

    private fun manager() = activity.getSystemService(DownloadManager::class.java)

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
            if (request.url.host == null) return null
            if (served(request.url)) return file(request)
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

        /** The object a page posts the bytes of a download it reads to. */
        const val READER = "octopusDownload"
    }
}
