import { useTranslation } from 'react-i18next'
import { useApp } from '@/store'
import { nearbyView } from '@/lib/syncView'
import SettingsNavRow, { type StatusTone } from '@/components/elements/SettingsNavRow'
import { NearbyGlyph } from '@/components/Main/icons'
import { useSubpage } from '../../sectionNav'

// Nearby sync's standing (`nearbyView`, which the chip reads too), whether or
// not a cloud account speaks for the vault: off, on, or a device that needs
// something done. Which device, and what, is said on its row inside.
export default function NearbyRow() {
  const { t } = useTranslation()
  const { open } = useSubpage()
  const nearby = useApp(state => state.nearby)

  // Only where this build can do it at all.
  if (!nearby?.available) return null

  const view = nearbyView(nearby)
  const [status, tone]: [string, StatusTone] = !view
    ? [t('Off'), 'neutral']
    : view.tone === 'bad'
      ? [t('Needs attention'), 'bad']
      : [t('On'), 'good']

  return (
    <SettingsNavRow
      icon={<NearbyGlyph size={16} />}
      iconActive={view !== null}
      label={t('Nearby devices')}
      description={t('Directly between your devices on the same network. No account needed.')}
      status={status}
      statusTone={tone}
      onClick={() => open({ key: 'sync-nearby' })}
      testid="settings-sync-open-nearby"
    />
  )
}
