import { useEffect } from 'react'
import { useTranslation } from 'react-i18next'
import { useApp } from '@/store'
import { isMobile } from '@/lib/platform'
import { cx } from '@/utils/cx'
import Note from '@/components/elements/Note'
import SettingsGroup from '@/components/elements/SettingsGroup'
import SettingsRow from '@/components/elements/SettingsRow'
import { CARD, CARD_OPEN, LABEL } from '@/components/elements/tokens'
import { LaptopGlyph, PhoneGlyph } from '@/components/Main/icons'
import { useSubpage } from '../../../sectionNav'
import SubpageScroller from '../../../SubpageScroller'
import Devices from './Devices'
import EnabledRow from './EnabledRow'

// Sync & backup › Nearby devices: the switch, the name this device goes by,
// and the devices it has synced with. A build without nearby sync has no page
// to show, so one that loses it (or never had it) steps back to the overview.
export default function NearbySubpage() {
  const { t } = useTranslation()
  const nearby = useApp(state => state.nearby)
  const { close } = useSubpage()
  const available = nearby?.available ?? false

  useEffect(() => {
    if (!available) close()
  }, [available, close])

  if (!nearby?.available) return null

  return (
    <SubpageScroller testid="settings-nearby">
      <section className="mb-7">
        <div className={CARD}>
          <EnabledRow enabled={nearby.enabled} seq={nearby.seq} />
        </div>
      </section>

      {nearby.enabled && (
        <>
          {nearby.deviceName && (
            <SettingsGroup label={t('This device')}>
              <SettingsRow
                icon={isMobile ? <PhoneGlyph /> : <LaptopGlyph />}
                label={nearby.deviceName}
                description={t('The name your other devices see')}
                testid="settings-nearby-this-device"
              />
            </SettingsGroup>
          )}

          {/* A SettingsGroup spelled out: CARD_OPEN lets a device row's menu
              hang past the last row. */}
          <section className="mb-7">
            <div className={cx(LABEL, 'mb-2')}>{t('Your devices')}</div>
            <div className={CARD_OPEN}>
              <Devices devices={nearby.devices} />
            </div>
            <Note className="px-1 pt-2.5">
              {t('Two devices sync the lockboxes they both hold, and nothing else.')}
            </Note>
          </section>
        </>
      )}
    </SubpageScroller>
  )
}
