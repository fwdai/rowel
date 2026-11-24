package app.rowel.mobile

import android.app.Activity
import android.util.Base64
import androidx.fragment.app.FragmentActivity
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
class StoreArgs {
  /** The key material, base64. */
  lateinit var material: String
}

/**
 * The Tauri side of [BiometricKey]: the four commands `biometric_key.rs` sends. No capability
 * names this plugin, so the webview cannot reach them; Rust is the only caller. Tauri runs
 * every command on the main thread, which is where [BiometricKey] has to be used, and a command
 * answers once the prompt it shows has been answered.
 *
 * Key material crosses as base64 in JSON. The decoded bytes are scrubbed once used; the strings
 * that carry them (the request and its `material`, the base64 of a retrieved key and the
 * response it goes out in) are immutable on the JVM and are left to the garbage collector.
 *
 * Instantiated by name from Rust (`register_android_plugin`); Tauri's own consumer ProGuard
 * rules keep every `@TauriPlugin` class, its constructor and its `@Command` methods, and every
 * `@InvokeArg` class, so a minified build still finds them.
 */
@TauriPlugin
class BiometricKeyPlugin(activity: Activity) : Plugin(activity) {
  // TauriActivity is an AppCompatActivity, which BiometricPrompt needs to put its fragment in.
  private val host = activity as FragmentActivity
  private val key = BiometricKey(activity.applicationContext)

  @Command
  fun status(invoke: Invoke) {
    val status = key.status()
    invoke.resolve(
      JSObject()
        .put("available", status.available)
        .put("fingerprint", status.fingerprint)
        .put("face", status.face)
        .put("iris", status.iris)
    )
  }

  @Command
  fun store(invoke: Invoke) {
    val material = Base64.decode(invoke.parseArgs(StoreArgs::class.java).material, Base64.DEFAULT)
    key.store(host, material) { outcome ->
      material.fill(0)
      when (outcome) {
        is BiometricKey.Outcome.Done -> invoke.resolve()
        is BiometricKey.Outcome.Refused -> reject(invoke, outcome)
      }
    }
  }

  @Command
  fun retrieve(invoke: Invoke) {
    key.retrieve(host) { outcome ->
      when (outcome) {
        is BiometricKey.Outcome.Done -> {
          val material = outcome.value
          // NO_WRAP: the default breaks lines, which Rust's decoder refuses.
          invoke.resolve(JSObject().put("material", Base64.encodeToString(material, Base64.NO_WRAP)))
          material.fill(0)
        }
        is BiometricKey.Outcome.Refused -> reject(invoke, outcome)
      }
    }
  }

  @Command
  fun remove(invoke: Invoke) {
    key.remove()
    invoke.resolve()
  }

  private fun reject(invoke: Invoke, refused: BiometricKey.Outcome.Refused) {
    invoke.reject(refused.message, refused.reason.code)
  }
}
