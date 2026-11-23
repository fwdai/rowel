import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { describeError } from '@/api/errors'
import { nearbySetEnabled } from '@/api/nearby'
import Note from '@/components/elements/Note'
import SettingsRow from '@/components/elements/SettingsRow'
import Toggle from '@/components/elements/Toggle'
import { NearbyGlyph } from '@/components/Main/icons'

interface Props {
  enabled: boolean
  seq: number
}

export default function EnabledRow({ enabled, seq }: Props) {
  const { t } = useTranslation()
  const [pressed, setPressed] = useState<{ enabled: boolean; seq: number } | null>(null)
  const [error, setError] = useState<string | null>(null)
  const label = t('Sync with nearby devices')
  // A press holds until a newer status, so the switch does not snap back before the event.
  const checked = pressed?.seq === seq ? pressed.enabled : enabled

  const toggle = (next: boolean) => {
    setPressed({ enabled: next, seq })
    setError(null)
    nearbySetEnabled(next).catch((reason: unknown) => {
      setPressed(null)
      setError(describeError(reason))
    })
  }

  return (
    <SettingsRow
      label={label}
      icon={<NearbyGlyph size={16} />}
      iconActive={checked}
      description={t(
        'Your devices on the same network sync directly while {{appName}} is open and unlocked on both. Nothing leaves your network.'
      )}
      control={
        <Toggle
          name="nearby"
          checked={checked}
          onChange={toggle}
          aria-label={label}
          testid="settings-nearby-toggle"
        />
      }
    >
      {error && (
        <Note tone="bad" testid="settings-nearby-error">
          {error}
        </Note>
      )}
    </SettingsRow>
  )
}
