package app.rowel.mobile

import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.os.Build
import android.os.PersistableBundle
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

// The value of ClipDescription.EXTRA_IS_SENSITIVE, a name the SDK only has
// from API 33. Android 13 and later keep a clip that carries it out of the
// preview shown after a copy; the key is the same string on the earlier
// versions that honour it, so it is written on all of them.
private const val IS_SENSITIVE = "android.content.extra.IS_SENSITIVE"

// Marks a clip as a secret this app copied. It rides in the clip's
// description, which can be read back without reading the text.
private const val IS_OURS = "app.rowel.mobile.extra.SECRET"

/**
 * The clipboard, for secrets: the Android half of `commands/clipboard.rs`,
 * which decides when each of these runs. The webview cannot call these: no
 * capability grants `secret-clipboard`, and Tauri refuses a plugin command
 * that none does.
 */
@TauriPlugin
class SecretClipboardPlugin(activity: Activity) : Plugin(activity) {
  private val manager =
    activity.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager

  @Command
  fun write(invoke: Invoke) {
    val clip = ClipData.newPlainText(null, invoke.getArgs().getString("text"))
    clip.description.extras = PersistableBundle().apply {
      putBoolean(IS_SENSITIVE, true)
      putBoolean(IS_OURS, true)
    }
    manager.setPrimaryClip(clip)
    invoke.resolve()
  }

  /**
   * Whether the clip on the clipboard is one `write` put there, as `ours`.
   * The key is left out when Android hands back no description: the clipboard
   * is empty, or this app may not see it (since Android 10, whenever it does
   * not have focus). Only the description is asked for; asking for the clip
   * itself is what Android 12 and later report to the user as a paste.
   */
  @Command
  fun peek(invoke: Invoke) {
    val seen = JSObject()
    manager.primaryClipDescription?.let {
      seen.put("ours", it.extras?.getBoolean(IS_OURS) == true)
    }
    invoke.resolve(seen)
  }

  /**
   * Empties the clipboard whatever it holds, without asking first whether it
   * holds anything: a write needs no focus, but `hasPrimaryClip` answers "no"
   * to an app that has none.
   */
  @Command
  fun clear(invoke: Invoke) {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
      manager.clearPrimaryClip()
    } else {
      // Before API 28 there is nothing to clear with: an empty clip takes the
      // secret's place.
      manager.setPrimaryClip(ClipData.newPlainText(null, ""))
    }
    invoke.resolve()
  }
}
