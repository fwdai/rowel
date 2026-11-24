import type { TKey } from '@/i18n'
import type { BiometryType } from '@/api/types'
import { isAndroid } from '@/lib/platform'

// Android has no brand names for its gates, and names one only when the device
// has a single sensor it can be (`biometrics.rs`), so its `'none'` is a real
// answer — several sensors, and the system prompt picks — not a placeholder.
const ANDROID: Record<BiometryType, TKey> = {
  face: 'Face unlock',
  touch: 'Fingerprint',
  none: 'Biometrics'
}

/**
 * What the OS biometric gate is called on *this device*. The glyph that goes
 * with the name is `<BiometryGlyph>` (components/elements/BiometryGlyph.tsx).
 *
 * Asked of the backend (`biometry_type`, over `LAContext.biometryType`) rather
 * than derived from the build: iPhones and Touch ID iPads run the same iOS
 * binary, so a compile-time `isIOS` labelled the iPads Face ID.
 *
 * Off Android, `'none'` falls to the fingerprint pair. It only reaches a call
 * site when the device has no biometry at all — in which case nothing
 * biometric is drawn — or before the probe has answered, where the desktop's
 * historical name is the safer placeholder.
 */
export const biometryLabel = (type: BiometryType): TKey =>
  isAndroid ? ANDROID[type] : type === 'face' ? 'Face ID' : 'Touch ID'
