import { useTranslation } from 'react-i18next'
import { useApp } from '@/store'
import { cx } from '@/utils/cx'
import { CARD_OPEN, LABEL } from '@/components/elements/tokens'
import Devices from './Devices'
import EnabledRow from './EnabledRow'

/**
 * The owner's devices on the same network, syncing with this one directly.
 * It runs beside the cloud account rather than as one of its providers — a
 * lockbox may use both — so it is a group of its own and leaves the account
 * card alone.
 *
 * Drawn only once the backend has said this build can do it at all; until its
 * status arrives there is nothing true to show.
 *
 * A SettingsGroup, spelled out: the card's overflow stays open so a device
 * row's menu can hang past the last row, as on Settings › Lockboxes.
 */
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
