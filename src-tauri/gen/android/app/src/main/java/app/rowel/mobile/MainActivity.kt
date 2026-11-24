package app.rowel.mobile

import android.os.Bundle
import android.view.WindowManager
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    // Keeps the vault out of screenshots, screen recordings and the recents
    // thumbnail. Set before the window has drawn anything.
    window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
    super.onCreate(savedInstanceState)
  }

  override fun onWebViewCreate(webView: WebView) = ShellBridge.attach(this, webView)
}
