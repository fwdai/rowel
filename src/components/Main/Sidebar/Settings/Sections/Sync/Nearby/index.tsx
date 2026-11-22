import { useTranslation } from 'react-i18next'
import { useApp } from '@/store'
import { cx } from '@/utils/cx'
import { CARD_OPEN, LABEL } from '@/components/elements/tokens'
import Devices from './Devices'
import EnabledRow from './EnabledRow'

// A SettingsGroup spelled out: CARD_OPEN lets a device row's menu hang past the last row.
export default function Nearby() {
  const { t } = useTranslation()
  const nearby = useApp(state => state.nearby)

  if (!nearby?.available) return null

  return (
    <section className="mb-7" data-testid="settings-nearby">
      <div className={cx(LABEL, 'mb-2')}>{t('Nearby devices')}</div>
      <div className={CARD_OPEN}>
        <EnabledRow enabled={nearby.enabled} seq={nearby.seq} />
        {nearby.enabled && <Devices devices={nearby.devices} />}
      </div>
    </section>
  )
}
