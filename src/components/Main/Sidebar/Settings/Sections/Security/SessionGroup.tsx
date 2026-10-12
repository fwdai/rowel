import { useTranslation } from 'react-i18next'
import type { TFunction } from 'i18next'
import { usePrefs, setPref } from '@/store'
import { isMobile } from '@/lib/platform'
import SettingsGroup from '@/components/elements/SettingsGroup'
import SettingsRow from '@/components/elements/SettingsRow'
import Segmented from '@/components/elements/Segmented'
import { ClipboardGlyph, LockGlyph } from '@/components/Main/icons'
import HideOnCopyRow from './HideOnCopyRow'

const LOCK_OPTIONS = [
  { value: '60', label: '1 m' },
  { value: '300', label: '5 m' },
  { value: '900', label: '15 m' },
  { value: '1800', label: '30 m' },
  { value: '3600', label: '1 h' }
]

// Built per render so a locale switch retranslates. The unit values read the
// same in every locale; only `Never` is words.
const clipboardOptions = (t: TFunction) => [
  { value: '15000', label: '15 s' },
  { value: '30000', label: '30 s' },
  { value: '60000', label: '60 s' },
  { value: '0', label: t('Never') }
]

export default function SessionGroup() {
  const { t } = useTranslation()
  const lock = usePrefs(state => state.autolockSecs)
  const clipboard = usePrefs(state => state.clipboardTimeoutMs)

  // Rust re-arms the auto-lock itself when this preference lands (`set_settings`).
  const onLock = (value: string) => setPref('autolockSecs', Number(value))

  const onClipboard = (value: string) => {
    setPref('clipboardTimeoutMs', Number(value))
  }

  // The row label doubles as the radiogroup's accessible name.
  const lockLabel = t('Lock vault after')
  const clipboardLabel = t('Clear clipboard')

  return (
    <SettingsGroup label={t('Auto-lock')}>
      <SettingsRow
        label={lockLabel}
        icon={<LockGlyph size={16} />}
        description={t('Idle time before the vault seals itself')}
        control={
          <Segmented
            name={lockLabel}
            options={LOCK_OPTIONS}
            value={String(lock)}
            onChange={onLock}
            testidPrefix="settings-autolock"
          />
        }
      />
      <SettingsRow
        label={clipboardLabel}
        icon={<ClipboardGlyph />}
        description={t('Copied secrets are wiped after this delay')}
        control={
          <Segmented
            name={clipboardLabel}
            options={clipboardOptions(t)}
            value={String(clipboard)}
            onChange={onClipboard}
            testidPrefix="settings-clipboard"
          />
        }
      />
      {!isMobile && <HideOnCopyRow />}
    </SettingsGroup>
  )
}
