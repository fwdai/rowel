import { useTranslation } from 'react-i18next'
import type { TFunction } from 'i18next'
import type { BrowserStatus } from '@/api/browser'
import SettingsGroup from '@/components/elements/SettingsGroup'
import SettingsRow from '@/components/elements/SettingsRow'
import { GlobeGlyph } from '@/components/Main/icons'

type Browser = BrowserStatus['browsers'][number]

// Whether the manifest is in place only says something once the host is on:
// turning it on is what writes one, for each browser found here. A place
// KeePassXC already holds is said either way, since turning on will not take it.
const state = (t: TFunction, browser: Browser, enabled: boolean) => {
  if (!browser.detected) return t('Not found')
  if (browser.conflict) return t('Registered to KeePassXC')
  if (!enabled) return t('Detected')
  return browser.installed ? t('Ready') : t('Could not be set up')
}

export default function Browsers({
  browsers,
  enabled
}: {
  browsers: Browser[]
  enabled: boolean
}) {
  const { t } = useTranslation()

  return (
    <SettingsGroup label={t('Browsers')}>
      {browsers.map(browser => (
        <SettingsRow
          key={browser.id}
          testid={`settings-browser-${browser.id}`}
          label={browser.label}
          icon={<GlobeGlyph />}
          iconActive={enabled && browser.installed}
          description={state(t, browser, enabled)}
        />
      ))}
    </SettingsGroup>
  )
}
