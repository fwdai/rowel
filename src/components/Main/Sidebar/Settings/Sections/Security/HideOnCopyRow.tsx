import { useTranslation } from 'react-i18next'
import { usePrefs, setPref } from '@/store'
import SettingsRow from '@/components/elements/SettingsRow'
import Toggle from '@/components/elements/Toggle'
import { HideGlyph } from '@/components/Main/icons'

// After a copy, Rowel gets out of the way so the paste lands in the app the
// value is for (`services/copy.ts`). Desktop only: the caller leaves it off a
// phone, which has no window to send back.
export default function HideOnCopyRow() {
  const { t } = useTranslation()
  const hideOnCopy = usePrefs(state => state.hideOnCopy)
  const label = t('Hide Rowel after copying')

  return (
    <SettingsRow
      label={label}
      icon={<HideGlyph />}
      iconActive={hideOnCopy}
      description={t('Go back to the app you came from once a value is copied')}
      control={
        <Toggle
          name="hideOnCopy"
          checked={hideOnCopy}
          onChange={next => setPref('hideOnCopy', next)}
          aria-label={label}
          testid="settings-hide-on-copy-toggle"
        />
      }
    />
  )
}
