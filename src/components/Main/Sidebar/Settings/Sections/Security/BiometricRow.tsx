import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { TFunction } from 'i18next'
import type { BiometricMode } from '@/api/types'
import { enableBiometric, disableBiometric } from '@/api/auth'
import { describeError } from '@/api/errors'
import { useApp, refreshApp } from '@/store'
import { isMobile } from '@/lib/platform'
import { biometryLabel } from '@/lib/biometry'
import Note from '@/components/elements/Note'
import SettingsRow from '@/components/elements/SettingsRow'
import Toggle from '@/components/elements/Toggle'
import BiometryGlyph from '@/components/elements/BiometryGlyph'

// What the gate actually is, once we know it. Before enrollment we can only
// describe the offer; afterwards the recorded mode says which guarantee holds.
const description = (t: TFunction, mode: BiometricMode | null) => {
  if (mode === 'protected')
    return t(
      'Your vault key is protected by the Secure Enclave and invalidated if your fingerprints change.'
    )
  if (mode === 'prompt')
    return t(
      'Your vault key is stored in your OS credential store and released after a biometric check.'
    )
  return t(
    'Store your vault key in the OS secure store, released only after a biometric check.'
  )
}

export default function BiometricRow() {
  const { t } = useTranslation()
  // The launch probe owns both halves of this row; the failure is ours alone.
  const biometric = useApp(state => state.status?.biometric)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  // One enrollment opens every workspace, and it stores the primary's key — so
  // a workspace opened on its own password, with the primary still closed this
  // session, has nothing to enrol with yet. The probe says so (`canEnroll`),
  // and the switch waits rather than offering a press that would be refused.
  const enrollable = !!biometric?.available || !!biometric?.canEnroll

  const toggle = () => {
    setBusy(true)
    setError(null)
    const op = biometric?.available ? disableBiometric() : enableBiometric()
    op.catch((err: unknown) => setError(describeError(err)))
      // Whichever way it went, the row is redrawn from a fresh probe rather
      // than from a guess: an unentitled build settles on a different mode
      // than it was offered, and a refusal changes nothing at all.
      .finally(() => refreshApp().finally(() => setBusy(false)))
  }

  // A phone names the gate it has — Face ID or Touch ID, as the lock screen
  // does; the desktop keeps naming the two it may have.
  const type = biometric?.type ?? 'touch'
  const label = isMobile
    ? t('Unlock with {{biometry}}', { biometry: t(biometryLabel(type)) })
    : t('Unlock with Touch ID or Windows Hello')

  // A row of the Unlock card: its tile lights once the key is enrolled.
  return (
    <SettingsRow
      label={label}
      icon={<BiometryGlyph type={type} size={16} />}
      iconActive={!!biometric?.available}
      description={description(t, biometric?.available ? biometric.mode : null)}
      control={
        <Toggle
          name="biometric"
          checked={!!biometric?.available}
          disabled={busy || !enrollable}
          onChange={toggle}
          aria-label={label}
          testid="settings-biometric-toggle"
        />
      }
    >
      {error && (
        <Note tone="bad" testid="settings-biometric-error">
          {error}
        </Note>
      )}
    </SettingsRow>
  )
}
