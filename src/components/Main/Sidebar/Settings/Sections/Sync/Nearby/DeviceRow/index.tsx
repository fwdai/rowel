import { useState } from 'react'
import { describeError } from '@/api/errors'
import { nearbyPause, nearbyRemove, type NearbyDevice } from '@/api/nearby'
import Note from '@/components/elements/Note'
import SettingsRow from '@/components/elements/SettingsRow'
import Menu from './Menu'
import PlatformGlyph from './PlatformGlyph'
import RemoveConfirm from './RemoveConfirm'
import Status from './Status'

export default function DeviceRow({ device }: { device: NearbyDevice }) {
  const [removing, setRemoving] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const run = (command: () => Promise<void>) => {
    setError(null)
    command().catch((reason: unknown) => setError(describeError(reason)))
  }

  const remove = () => {
    setRemoving(false)
    run(() => nearbyRemove(device.id))
  }

  return (
    <>
      <SettingsRow
        testid={`settings-nearby-device-${device.id}`}
        icon={<PlatformGlyph platform={device.platform} />}
        iconActive={device.state === 'upToDate' || device.state === 'syncing'}
        label={device.name}
        description={<Status device={device} />}
        control={
          <Menu
            id={device.id}
            paused={device.state === 'paused'}
            onPause={paused => run(() => nearbyPause(device.id, paused))}
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
