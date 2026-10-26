import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { describeError } from '@/api/errors'
import { orgApproveDevice, type OrgDevice } from '@/api/org'
import { cx } from '@/utils/cx'
import Button from '@/components/elements/Button'
import Chip from '@/components/elements/Chip'
import ConfirmSheet from '@/components/elements/ConfirmSheet'
import Note from '@/components/elements/Note'
import { LABEL } from '@/components/elements/tokens'
import type { Listed } from './useDetails'
import { deviceStateTone, deviceStateWords } from './words'

const APPROVE_TITLE_ID = 'org-approve-title'

interface Props {
  url: string
  devices: Listed<OrgDevice>
  /** The lists are read again once an approval went through. */
  onChanged: () => void
}

/**
 * The member's devices as the server lists them. Approving one is this
 * device's job as much as an admin's — and the one check that makes it safe
 * is the fingerprint: the confirm quotes the one the server reports, for the
 * person to compare with what the other device shows.
 */
export default function Devices({ url, devices, onChanged }: Props) {
  const { t } = useTranslation()
  const [approving, setApproving] = useState<OrgDevice | null>(null)
  const [failure, setFailure] = useState<string | null>(null)

  const approve = () => {
    const device = approving
    setApproving(null)
    if (!device) return
    orgApproveDevice(url, device.id).then(onChanged, (error: unknown) =>
      setFailure(describeError(error))
    )
  }

  return (
    <div data-testid="org-devices">
      <div className={cx(LABEL, 'mb-1.5')}>{t('Devices')}</div>
      {devices.items.length === 0 && !devices.error && (
        <Note testid="org-devices-empty">{t('The server lists no devices for you yet')}</Note>
      )}
      {devices.items.map(device => (
        <div
          key={device.id}
          data-testid={`org-device-${device.id}`}
          className="flex flex-wrap items-center gap-x-3 gap-y-1 py-1.5"
        >
          <span className="text-base text-text">{device.name}</span>
          {device.thisDevice && <Chip tone="accent">{t('This device')}</Chip>}
          <Chip tone={deviceStateTone(device.state)}>{deviceStateWords(t, device.state)}</Chip>
          <code className="font-mono text-sm text-text2">{device.fingerprint}</code>
          {device.state === 'pending' && !device.thisDevice && (
            <Button
              variant="pale"
              size="md"
              testid={`org-approve-${device.id}`}
              onClick={() => setApproving(device)}
            >
              {t('Approve')}
            </Button>
          )}
        </div>
      ))}
      {(devices.error || failure) && (
        <Note tone="bad" testid="org-devices-error" className="mt-1">
          {failure ?? devices.error}
        </Note>
      )}
      {approving && (
        <ConfirmSheet
          id={APPROVE_TITLE_ID}
          tone="accent"
          testid="org-approve-sheet"
          actionTestid="org-approve-confirm"
          title={t('Approve “{{name}}”?', { name: approving.name })}
          message={t('Only approve it if the key that device shows reads {{fingerprint}}.', {
            fingerprint: approving.fingerprint
          })}
          action={t('Approve device')}
          onConfirm={approve}
          onClose={() => setApproving(null)}
        />
      )}
    </div>
  )
}
