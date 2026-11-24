package app.rowel.mobile

import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyPermanentlyInvalidatedException
import android.security.keystore.KeyProperties
import androidx.biometric.BiometricManager
import androidx.biometric.BiometricPrompt
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity
import java.io.File
import java.io.IOException
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * The vault key material, sealed under an Android Keystore key that only a strong biometric
 * can use.
 *
 * Two things are kept, and neither is the key material. The wrapping key is AES-256, generated
 * inside the Android Keystore and never readable from it; it requires biometric authentication
 * for every single use, and Android destroys it when a biometric is enrolled, the last one is
 * removed, or the screen lock is turned off. The key material sealed under it (AES-GCM: the IV,
 * then the ciphertext and tag) is a file in the app's no-backup directory. The file is private
 * to the app, and inert on its own: opening it takes the wrapping key, which the Keystore
 * releases to a cipher only after [BiometricPrompt] has authorized that cipher. A file rather
 * than a preference because it is binary and has to stay out of Auto Backup — restored onto
 * another device, or after a reinstall, it would be a blob whose key no longer exists.
 *
 * A plain class with no Tauri in it: [BiometricKeyPlugin] is one caller, and anything else in
 * the app that has an activity to show the prompt on can be another. Every method is called on
 * the main thread, which is where [BiometricPrompt] has to be used, and every answer comes back
 * on it.
 *
 * `secure_store.rs` records this gate as `GateMode::KeystoreKey`.
 */
class BiometricKey(private val context: Context) {

  /** Why an operation did not produce its result. The codes are the wire to `biometric_key.rs`. */
  enum class Reason(val code: String) {
    /** The prompt ended without an answer: dismissed, timed out, or taken down by the system. */
    CANCELLED("cancelled"),

    /** Nothing is stored. */
    NOT_FOUND("notFound"),

    /** Android destroyed the wrapping key: the enrolled biometrics or the screen lock changed. */
    INVALIDATED("invalidated"),

    /** The device has a sensor and nothing enrolled on it. */
    NONE_ENROLLED("noneEnrolled"),

    /** Too many failed attempts; biometrics are refused for a while, or until the device is unlocked. */
    LOCKOUT("lockout"),

    /** No strong biometric sensor, or not one that can be used right now. */
    UNAVAILABLE("unavailable"),

    /** Anything else. */
    FAILED("failed"),
  }

  sealed class Outcome<out T> {
    class Done<T>(val value: T) : Outcome<T>()

    class Refused(val reason: Reason, val message: String) : Outcome<Nothing>()
  }

  /**
   * [available] is whether a strong biometric can authenticate right now. The rest is which
   * sensors the device declares — not what is enrolled, and not what the prompt will ask for,
   * neither of which Android tells an app.
   */
  class Status(
    val available: Boolean,
    val fingerprint: Boolean,
    val face: Boolean,
    val iris: Boolean,
  )

  private val blob = File(context.noBackupFilesDir, BLOB_FILE)

  // Whether a prompt is up. A second request made while one is showing is not one androidx
  // promises to answer, and an unanswered request leaves its caller waiting for good, so it is
  // refused instead. Main thread only, like everything else here.
  private var prompting = false

  fun status(): Status =
    Status(
      available = canAuthenticate() == BiometricManager.BIOMETRIC_SUCCESS,
      fingerprint = hasFeature(PackageManager.FEATURE_FINGERPRINT),
      // The face and iris features were given names in Android 10.
      face = Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q && hasFeature(PackageManager.FEATURE_FACE),
      iris = Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q && hasFeature(PackageManager.FEATURE_IRIS),
    )

  /**
   * Seal [material] under a fresh wrapping key, replacing whatever was stored. Shows the prompt:
   * encrypting is a use of the key like any other. A refusal before the prompt could be shown
   * leaves what was stored; any later one leaves nothing.
   *
   * The caller owns [material] and scrubs it once [done] has run.
   */
  fun store(activity: FragmentActivity, material: ByteArray, done: (Outcome<Unit>) -> Unit) {
    // Before anything is deleted: a request that comes while a prompt is showing must not take
    // that prompt's key from under it. And generating a key that requires a biometric fails, with
    // an exception that does not say why, on a device where none is enrolled.
    (cannotPrompt(activity) ?: cannotAuthenticate())?.let {
      done(it)
      return
    }
    val cipher =
      try {
        remove()
        // No IV is passed: the Keystore insists on choosing it.
        Cipher.getInstance(TRANSFORMATION).apply { init(Cipher.ENCRYPT_MODE, generateKey()) }
      } catch (e: Exception) {
        removeQuietly()
        done(failed(e))
        return
      }
    prompt(activity, cipher, { authorized -> write(authorized.iv + authorized.doFinal(material)) }) { outcome ->
      // A key with no blob beside it would be an enrollment that can never be read.
      if (outcome is Outcome.Refused) removeQuietly()
      done(outcome)
    }
  }

  /** Unseal the stored key material. Shows the prompt. The caller scrubs the bytes it is handed. */
  fun retrieve(activity: FragmentActivity, done: (Outcome<ByteArray>) -> Unit) {
    cannotPrompt(activity)?.let {
      done(it)
      return
    }
    val (contents, cipher) =
      try {
        val key = loadKey()
        val stored = if (blob.exists()) blob.readBytes() else null
        if (key == null || stored == null || stored.size <= IV_BYTES) {
          // Half an enrollment is none: whichever half is left goes too.
          remove()
          done(Outcome.Refused(Reason.NOT_FOUND, "no biometric key is stored"))
          return
        }
        stored to
          Cipher.getInstance(TRANSFORMATION).apply {
            init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(TAG_BITS, stored, 0, IV_BYTES))
          }
      } catch (e: KeyPermanentlyInvalidatedException) {
        // The key is gone for good, so the blob it sealed can never be opened: both are cleared
        // rather than left for an unlock that cannot happen.
        removeQuietly()
        done(Outcome.Refused(Reason.INVALIDATED, e.toString()))
        return
      } catch (e: Exception) {
        // Not proof that the key is gone, so the enrollment stays for the next try.
        done(failed(e))
        return
      }
    prompt(activity, cipher, { authorized -> authorized.doFinal(contents, IV_BYTES, contents.size - IV_BYTES) }, done)
  }

  /** Delete the wrapping key and the sealed blob. Nothing stored is not an error. */
  fun remove() {
    val keyStore = keyStore()
    if (keyStore.containsAlias(KEY_ALIAS)) keyStore.deleteEntry(KEY_ALIAS)
    blob.delete()
  }

  // On a path that is already answering with a refusal of its own.
  private fun removeQuietly() {
    try {
      remove()
    } catch (_: Exception) {}
  }

  // Through a sibling and a rename, so the file is always a whole blob: one cut short would fail
  // GCM at every unlock, an enrollment only disabling it could clear.
  private fun write(bytes: ByteArray) {
    val partial = File(blob.path + ".partial")
    partial.writeBytes(bytes)
    if (!partial.renameTo(blob)) throw IOException("could not move the sealed key into place")
  }

  private fun failed(e: Exception) = Outcome.Refused(Reason.FAILED, e.toString())

  private fun hasFeature(name: String): Boolean = context.packageManager.hasSystemFeature(name)

  // Strong (Class 3) only: it is the one class the Keystore accepts for a key that requires
  // authentication for every use.
  private fun canAuthenticate(): Int =
    BiometricManager.from(context).canAuthenticate(BiometricManager.Authenticators.BIOMETRIC_STRONG)

  private fun cannotAuthenticate(): Outcome.Refused? =
    when (canAuthenticate()) {
      BiometricManager.BIOMETRIC_SUCCESS -> null
      BiometricManager.BIOMETRIC_ERROR_NONE_ENROLLED ->
        Outcome.Refused(Reason.NONE_ENROLLED, "no biometric is enrolled on this device")
      else -> Outcome.Refused(Reason.UNAVAILABLE, "no strong biometric is available on this device")
    }

  private fun cannotPrompt(activity: FragmentActivity): Outcome.Refused? =
    when {
      prompting -> Outcome.Refused(Reason.FAILED, "a biometric prompt is already showing")
      // BiometricPrompt drops a request made after the activity has saved its state, without
      // telling its callback — and the caller would wait for an answer that never comes.
      activity.supportFragmentManager.isStateSaved ->
        Outcome.Refused(Reason.CANCELLED, "the app is not in the foreground")
      else -> null
    }

  private fun keyStore(): KeyStore = KeyStore.getInstance(KEYSTORE).apply { load(null) }

  private fun loadKey(): SecretKey? = keyStore().getKey(KEY_ALIAS, null) as SecretKey?

  private fun generateKey(): SecretKey {
    val spec =
      KeyGenParameterSpec.Builder(KEY_ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
        .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
        .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
        .setKeySize(KEY_BITS)
        .setUserAuthenticationRequired(true)
        .setInvalidatedByBiometricEnrollment(true)
    // Android 11 lets a key say which authenticators unlock it, and for how long: a strong
    // biometric, for one use. Before that there is nothing to say — a key that requires
    // authentication and names no validity period is already per-use and biometric-only.
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
      spec.setUserAuthenticationParameters(0, KeyProperties.AUTH_BIOMETRIC_STRONG)
    }
    return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEYSTORE).run {
      init(spec.build())
      generateKey()
    }
  }

  // Shows the system prompt bound to `cipher` and, once it has authorized it, hands `done` what
  // `use` made with it. The cipher the prompt authorized is this same instance: the Keystore
  // binds the authorization to its operation. `done` is called once.
  private fun <T> prompt(
    activity: FragmentActivity,
    cipher: Cipher,
    use: (Cipher) -> T,
    done: (Outcome<T>) -> Unit,
  ) {
    val finish = { outcome: Outcome<T> ->
      prompting = false
      done(outcome)
    }
    val callback =
      object : BiometricPrompt.AuthenticationCallback() {
        override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
          finish(
            try {
              Outcome.Done(use(cipher))
            } catch (e: Exception) {
              failed(e)
            }
          )
        }

        override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
          finish(Outcome.Refused(reasonOf(errorCode), errString.toString()))
        }

        // onAuthenticationFailed is a finger or face that did not match: the prompt stays up
        // and says so itself, and how it ends arrives through one of the two above.
      }
    prompting = true
    try {
      val info =
        BiometricPrompt.PromptInfo.Builder()
          .setTitle(activity.getString(R.string.biometric_prompt_title))
          // Required whenever the device credential is not an allowed way in.
          .setNegativeButtonText(activity.getString(android.R.string.cancel))
          .setAllowedAuthenticators(BiometricManager.Authenticators.BIOMETRIC_STRONG)
          .build()
      BiometricPrompt(activity, ContextCompat.getMainExecutor(activity), callback)
        .authenticate(info, BiometricPrompt.CryptoObject(cipher))
    } catch (e: Exception) {
      finish(failed(e))
    }
  }

  private fun reasonOf(errorCode: Int): Reason =
    when (errorCode) {
      BiometricPrompt.ERROR_USER_CANCELED,
      BiometricPrompt.ERROR_NEGATIVE_BUTTON,
      BiometricPrompt.ERROR_CANCELED,
      BiometricPrompt.ERROR_TIMEOUT -> Reason.CANCELLED
      BiometricPrompt.ERROR_LOCKOUT,
      BiometricPrompt.ERROR_LOCKOUT_PERMANENT -> Reason.LOCKOUT
      BiometricPrompt.ERROR_NO_BIOMETRICS -> Reason.NONE_ENROLLED
      BiometricPrompt.ERROR_HW_NOT_PRESENT,
      BiometricPrompt.ERROR_HW_UNAVAILABLE -> Reason.UNAVAILABLE
      else -> Reason.FAILED
    }

  private companion object {
    const val KEYSTORE = "AndroidKeyStore"
    const val KEY_ALIAS = "vault-key-biometric"
    const val BLOB_FILE = "vault-key-biometric"
    const val TRANSFORMATION = "AES/GCM/NoPadding"
    const val KEY_BITS = 256
    const val IV_BYTES = 12
    const val TAG_BITS = 128
  }
}
