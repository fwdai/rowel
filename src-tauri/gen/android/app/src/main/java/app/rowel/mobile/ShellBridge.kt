package app.rowel.mobile

import android.app.Activity
import android.view.View
import android.view.ViewGroup
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat

/**
 * The screen around the webview, as the page sees it: `window.__ROWEL_ANDROID__`,
 * read by `src/lib/android.ts`. Three things only the activity can know or do.
 *
 * The insets. The activity is edge to edge, so the page is laid out under the
 * status bar, the navigation bar and any display cutout, and has to keep clear
 * of them itself. Whether Android System WebView reports them through
 * `env(safe-area-inset-*)` depends on its version, so they are measured here
 * and handed over as numbers instead, and the webview is given none of its own:
 * the page's answer is the same on every version.
 *
 * The keyboard. Edge to edge, the window no longer shrinks for it either, so
 * the webview is lifted clear of it here, by the keyboard's height: the page's
 * viewport then ends where the keys begin, as it would in a browser, and the
 * shell sized to it keeps the focused field in view. While it is up the
 * keyboard covers the bottom edge, so the bottom insets read zero.
 *
 * The bars' icons, which follow the app's own theme rather than the phone's,
 * and leaving the app from its root, which Tauri's back-button listener takes
 * away from the system (see `useBackButton`).
 *
 * The methods marked [JavascriptInterface] run on the webview's bridge thread,
 * so the insets are kept in a volatile string and the rest is posted to the
 * main thread. `proguard-rules.pro` keeps them by name.
 */
class ShellBridge private constructor(
  private val activity: Activity,
  private val webView: WebView,
) {
  @Volatile private var measured = "{}"

  /** The insets in device pixels, as JSON: `{ top, right, bottom, left, bar }`. */
  @JavascriptInterface
  fun insets(): String = measured

  @JavascriptInterface
  fun setDark(dark: Boolean) {
    activity.runOnUiThread {
      WindowCompat.getInsetsController(activity.window, activity.window.decorView).run {
        isAppearanceLightStatusBars = !dark
        isAppearanceLightNavigationBars = !dark
      }
    }
  }

  /** Into the background, as Back does on a launcher's root since Android 12. */
  @JavascriptInterface
  fun leave() {
    activity.runOnUiThread { activity.moveTaskToBack(true) }
  }

  private fun measure(view: View, windowInsets: WindowInsetsCompat): WindowInsetsCompat {
    val keyboard = windowInsets.getInsets(WindowInsetsCompat.Type.ime()).bottom
    val edges = windowInsets.getInsets(
      WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout()
    )
    // A three-button bar takes taps; a gesture handle does not, so this is
    // zero under one.
    val buttons = windowInsets.getInsets(WindowInsetsCompat.Type.tappableElement()).bottom

    (view.layoutParams as? ViewGroup.MarginLayoutParams)?.let { params ->
      if (params.bottomMargin != keyboard) {
        params.bottomMargin = keyboard
        view.layoutParams = params
      }
    }

    val bottom = if (keyboard > 0) 0 else edges.bottom
    val bar = if (keyboard > 0) 0 else buttons
    val next =
      """{"top":${edges.top},"right":${edges.right},"bottom":$bottom,"left":${edges.left},"bar":$bar}"""
    if (next != measured) {
      measured = next
      webView.evaluateJavascript("window.dispatchEvent(new Event('rowel:insets'))", null)
    }
    return WindowInsetsCompat.CONSUMED
  }

  companion object {
    /**
     * Before the first page loads (`WryActivity.onWebViewCreate`), so the page
     * finds the bridge from its first script, and on every reload after.
     */
    fun attach(activity: Activity, webView: WebView) {
      val bridge = ShellBridge(activity, webView)
      webView.addJavascriptInterface(bridge, "__ROWEL_ANDROID__")
      ViewCompat.setOnApplyWindowInsetsListener(webView) { view, insets -> bridge.measure(view, insets) }
    }
  }
}
