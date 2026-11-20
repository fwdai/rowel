import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { describeError } from '@/api/errors'
import { nearbyPause, nearbyRemove, type NearbyDevice } from '@/api/nearby'
import { useDates } from '@/hooks/useDates'
import Note from '@/components/elements/Note'
import SettingsRow from '@/components/elements/SettingsRow'
import Menu from './Menu'
import PlatformGlyph from './PlatformGlyph'
import RemoveConfirm from './RemoveConfirm'
import { statusLine } from './statusLine'

/**
 * One of the owner's other devices: what it is, its name, where it stands, and
 * Pause and Remove behind ⋯. Both answer through the next `nearby:status`, so
 * the row only keeps what failed. The tile is lit while the device is here and
 * talking to this one.
 */
export default function DeviceRow({ device }: { device: NearbyDevice }) {
  const { t } = useTranslation()
  const { relativeLong } = useDates()
  const [removing, setRemoving] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const line = statusLine(t, device, relativeLong)

  const fail = (reason: unknown) => setError(describeError(reason))

  const pause = (paused: boolean) => {
    setError(null)
    nearbyPause(device.id, paused).catch(fail)
  }

  const remove = () => {
    setRemoving(false)
    setError(null)
    nearbyRemove(device.id).catch(fail)
  }

  return (
    <>
      <SettingsRow
        testid={`settings-nearby-device-${device.id}`}
        icon={<PlatformGlyph platform={device.platform} />}
        iconActive={device.state === 'upToDate' || device.state === 'syncing'}
        label={device.name}
        description={
          <span
            data-testid={`settings-nearby-status-${device.id}`}
            className={line.bad ? 'text-bad' : undefined}
          >
            {line.text}
          </span>
        }
        control={
          <Menu
            id={device.id}
            paused={device.state === 'paused'}
            onPause={pause}
            onRemove={() => setRemoving(true)}
          />
        }
      >
        {error && <Note tone="bad">{error}</Note>}
      </SettingsRow>
      {removing && (
        <RemoveConfirm
          name={device.name}
          onConfirm={remove}
          onClose={() => setRemoving(false)}
        />
      )}
    </>
  )
}
