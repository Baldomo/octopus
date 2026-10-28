package com.zabealbe.octopus

import android.Manifest
import android.app.Activity
import android.content.res.Configuration
import android.os.Build
import android.provider.Settings
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import android.view.View
import android.webkit.WebView
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import app.tauri.PermissionState
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.Permission
import app.tauri.annotation.PermissionCallback
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

private const val MICROPHONE = "microphone"

@InvokeArg
class Keep {
    lateinit var pkcs8: String
}

@InvokeArg
class Bars {
    lateinit var appearance: String
}

/**
 * What Android keeps and shows for the app: the device key, sealed under an AES key the Keystore holds and never lets
 * leave the device, the device's name, the system bars' icons, the insets the page clears, and microphone access.
 */
@TauriPlugin(permissions = [Permission(strings = [Manifest.permission.RECORD_AUDIO], alias = MICROPHONE)])
class DevicePlugin(private val activity: Activity) : Plugin(activity) {
    /** Resolves to whether the app may record, asking the user the first time. */
    @Command
    fun microphone(invoke: Invoke) {
        if (getPermissionState(MICROPHONE) == PermissionState.GRANTED) return answerMicrophone(invoke)
        requestPermissionForAlias(MICROPHONE, invoke, "answerMicrophone")
    }

    @PermissionCallback
    private fun answerMicrophone(invoke: Invoke) {
        invoke.resolve(JSObject().put("granted", getPermissionState(MICROPHONE) == PermissionState.GRANTED))
    }

    /** Outside backups, which would carry the sealed key to a device whose Keystore cannot open it. */
    private val sealed = File(activity.noBackupFilesDir, "device.key.sealed")

    @Command
    fun kept(invoke: Invoke) {
        val kept = JSObject()
        if (sealed.exists()) kept.put("pkcs8", Base64.encodeToString(open(sealed.readBytes()), Base64.NO_WRAP))
        invoke.resolve(kept)
    }

    @Command
    fun keep(invoke: Invoke) {
        val pkcs8 = Base64.decode(invoke.parseArgs(Keep::class.java).pkcs8, Base64.NO_WRAP)
        sealed.writeBytes(seal(pkcs8))
        invoke.resolve()
    }

    @Command
    fun name(invoke: Invoke) {
        val name = Settings.Global.getString(activity.contentResolver, Settings.Global.DEVICE_NAME) ?: Build.MODEL
        invoke.resolve(JSObject().put("name", name))
    }

    private var appearance = "system"

    /** The status bar and display cutout's insets, in CSS pixels, which the page clears itself. */
    private var edges = JSObject().put("top", 0).put("left", 0).put("right", 0)

    /**
     * Edge to edge, the window runs under the status bar, the navigation bar and the on-screen keyboard. The webview
     * stays clear of the navigation bar on whichever side it sits and of the keyboard, and the page reads the rest
     * from `insets`.
     */
    override fun load(webView: WebView) {
        super.load(webView)
        page = webView
        val content = activity.findViewById<View>(android.R.id.content)
        ViewCompat.setOnApplyWindowInsetsListener(content) { view, insets ->
            val foot = insets.getInsets(WindowInsetsCompat.Type.ime() or WindowInsetsCompat.Type.navigationBars())
            view.setPadding(foot.left, 0, foot.right, foot.bottom)
            val sides = insets.getInsets(WindowInsetsCompat.Type.statusBars() or WindowInsetsCompat.Type.displayCutout())
            val density = activity.resources.displayMetrics.density
            // The padding already clears a side the navigation bar sits on.
            edges = JSObject().put("top", sides.top / density)
                .put("left", maxOf(0, sides.left - foot.left) / density)
                .put("right", maxOf(0, sides.right - foot.right) / density)
            WindowInsetsCompat.CONSUMED
        }
    }

    private var page: WebView? = null

    /**
     * The app is in front from onStart to onStop, which the page hears as `octopus-front`. A permission prompt pauses
     * the activity and hides the page from the webview's own `visibilitychange` while leaving the app in front.
     */
    override fun onStop() = front(false)

    override fun onRestart() = front(true)

    private fun front(shown: Boolean) {
        page?.evaluateJavascript("dispatchEvent(new CustomEvent('octopus-front', { detail: $shown }))", null)
    }

    @Command
    fun insets(invoke: Invoke) {
        invoke.resolve(edges)
    }

    /** Draws the status and navigation bars' icons to read against the page in the appearance the app shows. */
    @Command
    fun bars(invoke: Invoke) {
        appearance = invoke.parseArgs(Bars::class.java).appearance
        activity.runOnUiThread {
            drawBars()
            invoke.resolve()
        }
    }

    /** System follows the OS's appearance as it changes. */
    override fun onConfigurationChanged(newConfig: Configuration) {
        drawBars()
    }

    private fun drawBars() {
        val night = activity.resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK
        val dark = when (appearance) {
            "dark" -> true
            "light" -> false
            else -> night == Configuration.UI_MODE_NIGHT_YES
        }
        WindowCompat.getInsetsController(activity.window, activity.window.decorView).apply {
            isAppearanceLightStatusBars = !dark
            isAppearanceLightNavigationBars = !dark
        }
    }

    private fun sealingKey(): SecretKey {
        val store = KeyStore.getInstance(KEYSTORE).apply { load(null) }
        (store.getKey(ALIAS, null) as SecretKey?)?.let { return it }
        val spec = KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setKeySize(256)
            .build()
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEYSTORE).apply { init(spec) }.generateKey()
    }

    /** The cipher's IV, then the sealed bytes. */
    private fun seal(plain: ByteArray): ByteArray {
        val cipher = Cipher.getInstance(TRANSFORMATION).apply { init(Cipher.ENCRYPT_MODE, sealingKey()) }
        return cipher.iv + cipher.doFinal(plain)
    }

    private fun open(sealed: ByteArray): ByteArray {
        val iv = GCMParameterSpec(TAG_BITS, sealed, 0, IV_BYTES)
        val cipher = Cipher.getInstance(TRANSFORMATION).apply { init(Cipher.DECRYPT_MODE, sealingKey(), iv) }
        return cipher.doFinal(sealed, IV_BYTES, sealed.size - IV_BYTES)
    }

    private companion object {
        const val KEYSTORE = "AndroidKeyStore"
        const val ALIAS = "octopus-device-key"
        const val TRANSFORMATION = "AES/GCM/NoPadding"
        const val IV_BYTES = 12
        const val TAG_BITS = 128
    }
}
