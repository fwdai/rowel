use crate::error::Result;
use std::time::Duration;
use tauri::AppHandle;
// The delayed clear is everywhere but iOS, which neither writes through the
// plugin nor reads the pasteboard back — so none of it is compiled there.
#[cfg(not(target_os = "ios"))]
use std::sync::{Arc, Mutex, MutexGuard};
#[cfg(not(target_os = "ios"))]
use tauri::Manager;
#[cfg(not(target_os = "ios"))]
use tauri_plugin_clipboard_manager::ClipboardExt;

/// The pending clipboard clear.
///
/// One re-armable timer for the whole process rather than a thread that sleeps
/// for the delay: copying ten fields in a row used to park ten OS threads, each
/// holding on to the secret it was armed with. Last copy wins, which is also
/// what the compare-before-clear wants — an earlier value is no longer on the
/// clipboard, so there is nothing left for its deadline to clear.
///
/// Also remembers the last secret written, so a lock can clear it without
/// waiting for the deadline — or when there is no deadline at all ("Never").
///
/// Managed in `lib.rs`, and not on iOS: nothing arms it there.
#[cfg(not(target_os = "ios"))]
pub struct ClipboardClear {
    timer: Arc<crate::timer::Timer>,
    /// What this app last put on the clipboard, until it is cleared or the
    /// user copies something else over it. Zeroized on drop like every other
    /// copy of a secret this process holds. Its lock is also the lock every
    /// copy and every clear runs under, whole — see [`ClipboardClear::lock`].
    last: Mutex<Option<Remembered>>,
    /// Counts copies. Each remembered value carries the generation it was
    /// written under, and a timer callback acts only while its own generation
    /// is still the current one — see [`Remembered`].
    generation: std::sync::atomic::AtomicU64,
}

/// One copy: the value, and which copy it was.
///
/// The generation is what keeps an *earlier* copy's deadline from acting on a
/// *later* copy. The timer takes a callback out of its slot before running it,
/// so re-arming cannot cancel one already on its way: a copy that lands while
/// the previous deadline is firing would otherwise have its value forgotten by
/// that callback — and then left on the clipboard by the lock that came to
/// clear it, since nothing remembered it any more.
#[cfg(not(target_os = "ios"))]
struct Remembered {
    generation: u64,
    value: zeroize::Zeroizing<String>,
}

#[cfg(not(target_os = "ios"))]
impl Default for ClipboardClear {
    fn default() -> Self {
        Self {
            timer: crate::timer::Timer::spawn(),
            last: Mutex::new(None),
            generation: std::sync::atomic::AtomicU64::new(0),
        }
    }
}

#[cfg(not(target_os = "ios"))]
impl ClipboardClear {
    /// The lock a copy and a clear each hold for their whole run: the write
    /// and the remembering together, or the check of which copy is current,
    /// the read of the clipboard and the wipe together. The generation alone
    /// is not enough — checked and then acted on, it leaves a gap a copy can
    /// land in: the check said "still mine", the copy wrote the same secret
    /// again, the compare matched, and the new copy was wiped a whole TTL
    /// early. Under one lock the check and the wipe are one step, and the copy
    /// waits its turn.
    fn lock(&self) -> MutexGuard<'_, Option<Remembered>> {
        self.last.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Record `value` as the latest copy, returning its generation. `last` is
    /// the guard from [`ClipboardClear::lock`], so the caller has to hold it.
    fn remember(&self, last: &mut Option<Remembered>, value: &str) -> u64 {
        let generation = self
            .generation
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1;
        *last = Some(Remembered {
            generation,
            value: zeroize::Zeroizing::new(value.to_string()),
        });
        generation
    }
}

/// Whether the copy made under `generation` is still the latest.
#[cfg(not(target_os = "ios"))]
fn is_current(last: &Option<Remembered>, generation: u64) -> bool {
    last.as_ref().is_some_and(|r| r.generation == generation)
}

/// Forget the latest copy, but only if it is the one made under `generation`:
/// a later copy is someone else's to forget.
#[cfg(not(target_os = "ios"))]
fn forget_if(last: &mut Option<Remembered>, generation: u64) {
    if is_current(last, generation) {
        *last = None;
    }
}

/// The vault has just locked: a secret it copied must not outlive it.
///
/// Compare-before-clear, like the timed clear — the user may have copied
/// something of their own since, and that is theirs to keep. The pending timer
/// is dropped either way: the value it was armed for is gone or was never ours.
///
/// Not on iOS, where nothing is read back from the pasteboard (see
/// `copy_to_clipboard`): the expiry set at write time is the clear there.
pub fn clear_on_lock(app: &AppHandle) {
    #[cfg(not(target_os = "ios"))]
    {
        let state = app.state::<ClipboardClear>();
        let mut last = state.lock();
        state.timer.disarm();
        if let Some(written) = last.take().map(|r| r.value) {
            let current = app.clipboard().read_text().ok();
            if should_clear(&written, current.as_deref()) {
                let _ = app.clipboard().clear();
            }
        }
    }
    #[cfg(target_os = "ios")]
    let _ = app;
}

// Copy text to the clipboard, optionally clearing it after `clear_after_ms`.
//
// Secrets are written with the platform markers that keep them out of
// clipboard managers and history (concealed/transient types on macOS, the
// exclusion formats on Windows). On macOS the pasteboard is also prepared
// current-host-only, which is what keeps a copy off Universal Clipboard — the
// concealed marker alone does not: Handoff syncs it to the user's other
// devices like any other copy. The delayed
// clear is compare-before-clear: it only wipes the clipboard if it still holds
// the exact value we wrote, so a value the user copied afterwards is never lost.
#[tauri::command]
pub fn copy_to_clipboard(app: AppHandle, value: String, clear_after_ms: Option<u64>) -> Result<()> {
    let clear_after = clear_after_ms.map(Duration::from_millis);

    // iOS: the expiry set at write time is what clears the pasteboard there,
    // and it survives the app being suspended, which a timer thread does not.
    // Reading the pasteboard back to compare would also raise the iOS 16+
    // system paste banner over a value the user never asked to paste.
    #[cfg(target_os = "ios")]
    write_concealed(&app, &value, clear_after)?;

    #[cfg(not(target_os = "ios"))]
    {
        let state = app.state::<ClipboardClear>();
        // Held from before the write to after the arming, so a clear already
        // running cannot come between them (see `ClipboardClear::lock`).
        let mut last = state.lock();
        write_concealed(&app, &value, clear_after)?;
        // Remembered whether or not a deadline follows: with the timeout set to
        // "Never", the lock is the only clear this value will ever get.
        let generation = state.remember(&mut last, &value);
        if let Some(delay) = clear_after {
            let handle = app.clone();
            state.timer.arm(delay, move || {
                let state = handle.state::<ClipboardClear>();
                let mut last = state.lock();
                // A later copy owns the clipboard now, and its own deadline —
                // or the lock — will see to it. This one is done.
                if !is_current(&last, generation) {
                    return;
                }
                let current = handle.clipboard().read_text().ok();
                if should_clear(&value, current.as_deref()) {
                    let _ = handle.clipboard().clear();
                }
                // Cleared, or overwritten by the user: either way there is
                // nothing of ours left for a lock to find.
                forget_if(&mut last, generation);
            });
        } else {
            // A copy with no deadline replaces whatever an earlier one armed;
            // that value is no longer on the clipboard.
            state.timer.disarm();
        }
    }
    Ok(())
}

// Only clear when the clipboard still holds exactly what we wrote. Nothing but
// the delayed clear consults it, and iOS arms no such callback.
#[cfg_attr(target_os = "ios", allow(dead_code))]
fn should_clear(written: &str, current: Option<&str>) -> bool {
    current == Some(written)
}

/// The longest a secret may sit on the iOS pasteboard when the user's clipboard
/// timeout is "Never".
///
/// `ExpirationDate` is the only clear iOS gets: `clear_on_lock` is a no-op
/// there, because comparing before clearing means reading the pasteboard back,
/// which raises the iOS 16+ paste banner over a value the user never asked to
/// paste. Without an expiry, "Never" would leave the secret on the pasteboard
/// forever — across a lock, and past the point the user has forgotten it is
/// there. A day is long past any real use of a copied password, and still an
/// end.
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
const IOS_MAX_PASTEBOARD_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// The expiry every iOS pasteboard write carries: what the user asked for, or
/// the cap when they asked for nothing — and never longer than the cap.
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
fn ios_expiry(clear_after: Option<Duration>) -> Duration {
    clear_after
        .unwrap_or(IOS_MAX_PASTEBOARD_TTL)
        .min(IOS_MAX_PASTEBOARD_TTL)
}

#[cfg(target_os = "macos")]
fn write_concealed(_app: &AppHandle, value: &str, _clear_after: Option<Duration>) -> Result<()> {
    use objc2_app_kit::{NSPasteboard, NSPasteboardContentsOptions};
    use objc2_foundation::NSString;

    // public.utf8-plain-text == NSPasteboardTypeString.
    let text_type = NSString::from_str("public.utf8-plain-text");
    // The nspasteboard.org markers: concealed (a secret — don't show or keep
    // it) and transient (gone shortly — don't record it). Clipboard managers
    // and history read these; Universal Clipboard does not.
    let concealed_type = NSString::from_str("org.nspasteboard.ConcealedType");
    let transient_type = NSString::from_str("org.nspasteboard.TransientType");
    let text = NSString::from_str(value);

    let pb = NSPasteboard::generalPasteboard();
    // Clears the pasteboard like `clearContents`, and marks what is written
    // next as this Mac's alone, so Universal Clipboard does not hand the
    // secret to the user's other devices. Nothing may clear it again before
    // the writes — `declareTypes:owner:` does, and could take the option with
    // it — so the types are written straight onto the item this prepares.
    pb.prepareForNewContentsWithOptions(NSPasteboardContentsOptions::CurrentHostOnly);
    pb.setString_forType(&text, &text_type);
    pb.setString_forType(&text, &concealed_type);
    // A marker only: its presence is what is read, so it carries no copy of
    // the secret.
    pb.setString_forType(&NSString::from_str(""), &transient_type);
    Ok(())
}

#[cfg(target_os = "windows")]
fn write_concealed(app: &AppHandle, value: &str, _clear_after: Option<Duration>) -> Result<()> {
    // Fall back to a plain write if the native path can't take the clipboard.
    if write_windows(value).is_err() {
        app.clipboard()
            .write_text(value.to_string())
            .map_err(|e| crate::error::Error::Other(e.to_string()))?;
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn write_windows(value: &str) -> Result<()> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HANDLE, HGLOBAL};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
    };
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

    const CF_UNICODETEXT: u32 = 13;
    // Markers that opt the secret out of history, cloud sync, and monitoring.
    const EXCLUSIONS: [&str; 3] = [
        "ExcludeClipboardContentFromMonitorProcessing",
        "CanIncludeInClipboardHistory",
        "CanUploadToCloudClipboard",
    ];

    unsafe fn global_from_bytes(bytes: &[u8]) -> Option<HGLOBAL> {
        let handle = GlobalAlloc(GMEM_MOVEABLE, bytes.len()).ok()?;
        let ptr = GlobalLock(handle);
        if ptr.is_null() {
            return None;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as *mut u8, bytes.len());
        let _ = GlobalUnlock(handle);
        Some(handle)
    }

    unsafe {
        OpenClipboard(None).map_err(|e| crate::error::Error::Other(e.to_string()))?;
        let _ = EmptyClipboard();

        let mut utf16: Vec<u16> = value.encode_utf16().collect();
        utf16.push(0);
        let text_bytes: Vec<u8> = utf16.iter().flat_map(|u| u.to_ne_bytes()).collect();
        if let Some(handle) = global_from_bytes(&text_bytes) {
            let _ = SetClipboardData(CF_UNICODETEXT, Some(HANDLE(handle.0)));
        }

        for name in EXCLUSIONS {
            let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
            let format = RegisterClipboardFormatW(PCWSTR(wide.as_ptr()));
            if format != 0 {
                if let Some(handle) = global_from_bytes(&0u32.to_ne_bytes()) {
                    let _ = SetClipboardData(format, Some(HANDLE(handle.0)));
                }
            }
        }

        let _ = CloseClipboard();
    }
    Ok(())
}

// iOS has no "concealed" pasteboard type. What it does have is a per-write
// options dictionary: `LocalOnly` keeps the secret off Universal Clipboard (no
// other device ever sees it), and `ExpirationDate` has the system drop the item
// on its own, which is a stronger clear than a timer this process may never live
// to run. Every write gets an expiry — the user's, or the cap above when they
// asked for none.
#[cfg(target_os = "ios")]
fn write_concealed(_app: &AppHandle, value: &str, clear_after: Option<Duration>) -> Result<()> {
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSArray, NSDate, NSDictionary, NSNumber, NSString};
    use objc2_ui_kit::{
        UIPasteboard, UIPasteboardOption, UIPasteboardOptionExpirationDate,
        UIPasteboardOptionLocalOnly,
    };

    // public.utf8-plain-text is the UTI every iOS paste target reads text from.
    let uti = NSString::from_str("public.utf8-plain-text");
    let text = NSString::from_str(value);
    let text: &AnyObject = &text;
    let item = NSDictionary::from_slices(&[&*uti], &[text]);

    let local_only = NSNumber::numberWithBool(true);
    // Bound outside the vectors so they outlive the borrow the dictionary takes.
    let expires = NSDate::dateWithTimeIntervalSinceNow(ios_expiry(clear_after).as_secs_f64());
    let (local_only_key, expires_key) = unsafe {
        (
            UIPasteboardOptionLocalOnly,
            UIPasteboardOptionExpirationDate,
        )
    };
    let keys: Vec<&UIPasteboardOption> = vec![local_only_key, expires_key];
    let values: Vec<&AnyObject> = vec![&local_only, &expires];

    unsafe {
        UIPasteboard::generalPasteboard().setItems_options(
            &NSArray::from_slice(&[&*item]),
            &NSDictionary::from_slices(&keys, &values),
        );
    }
    Ok(())
}

#[cfg(not(any(target_vendor = "apple", target_os = "windows")))]
fn write_concealed(app: &AppHandle, value: &str, _clear_after: Option<Duration>) -> Result<()> {
    app.clipboard()
        .write_text(value.to_string())
        .map_err(|e| crate::error::Error::Other(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::{ios_expiry, should_clear, IOS_MAX_PASTEBOARD_TTL};
    use std::time::Duration;

    // "Never" on iOS means "until the pasteboard expires", not "forever": the
    // lock cannot clear it there, so the write-time expiry has to.
    #[test]
    fn never_still_expires_on_ios() {
        assert_eq!(ios_expiry(None), IOS_MAX_PASTEBOARD_TTL);
    }

    #[test]
    fn a_chosen_timeout_is_kept_and_bounded_by_the_cap() {
        assert_eq!(
            ios_expiry(Some(Duration::from_secs(30))),
            Duration::from_secs(30)
        );
        assert_eq!(
            ios_expiry(Some(Duration::from_secs(u32::MAX as u64))),
            IOS_MAX_PASTEBOARD_TTL
        );
    }

    #[test]
    fn clears_only_when_value_unchanged() {
        assert!(should_clear("secret", Some("secret")));
    }

    #[test]
    fn keeps_a_value_the_user_copied_afterwards() {
        assert!(!should_clear("secret", Some("something else")));
    }

    #[test]
    fn keeps_an_empty_or_unreadable_clipboard() {
        assert!(!should_clear("secret", None));
        assert!(!should_clear("secret", Some("")));
    }

    // The race the generation exists for: a copy's deadline fires while a
    // newer copy has already been remembered. The stale callback must leave
    // the newer value in place for its own deadline, or the lock, to clear.
    #[cfg(not(target_os = "ios"))]
    #[test]
    fn a_stale_deadline_does_not_forget_a_newer_copy() {
        let state = super::ClipboardClear::default();
        let mut last = state.lock();
        let first = state.remember(&mut last, "first");
        let second = state.remember(&mut last, "second");

        assert!(!super::is_current(&last, first));
        assert!(super::is_current(&last, second));

        super::forget_if(&mut last, first);
        assert_eq!(last.as_ref().map(|r| r.value.as_str()), Some("second"));
    }

    #[cfg(not(target_os = "ios"))]
    #[test]
    fn a_current_deadline_forgets_its_own_copy() {
        let state = super::ClipboardClear::default();
        let mut last = state.lock();
        let generation = state.remember(&mut last, "secret");

        super::forget_if(&mut last, generation);
        assert!(last.is_none());
    }
}
