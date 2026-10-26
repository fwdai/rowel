import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { describeError } from '@/api/errors'
import { orgDisconnect, serverHost, type OrgConnection } from '@/api/org'
import { loadOrgConnections } from '@/store'
import Button from '@/components/elements/Button'
import ConfirmSheet from '@/components/elements/ConfirmSheet'
import Note from '@/components/elements/Note'
import SettingsRow from '@/components/elements/SettingsRow'
import { ServerGlyph } from '@/components/Main/icons'
import DeviceLine from './DeviceLine'
import Devices from './Devices'
import Lockboxes from './Lockboxes'
import { useDetails } from './useDetails'

const DISCONNECT_TITLE_ID = 'org-disconnect-title'

interface Props {
  connection: OrgConnection
  /** This device's enrollment key, for the line that says approval is still waited for. */
  fingerprint: string | null
}

/**
 * One server the member is connected to: whose it is and as whom, where this
 * device stands with it, and under that its devices and lockboxes. Disconnect
 * is confirmed first, since it is the enrollment that goes: the org lockboxes
 * already here stay, and only stop syncing.
 */
export default function ConnectionCard({ connection, fingerprint }: Props) {
  const { t } = useTranslation()
  const { devices, lockboxes, reload } = useDetails(connection.url)
  const [confirming, setConfirming] = useState(false)
  const [failure, setFailure] = useState<string | null>(null)

  const disconnect = () => {
    setConfirming(false)
    orgDisconnect(connection.url).then(loadOrgConnections, (error: unknown) =>
      setFailure(describeError(error))
    )
  }

  return (
    <SettingsRow
      label={connection.tenantName}
      icon={<ServerGlyph />}
      iconActive={connection.deviceState === 'approved'}
      description={`${connection.memberEmail} · ${serverHost(connection.url)}`}
      testid={`org-connection-${serverHost(connection.url)}`}
      control={
        <Button
          variant="pale-danger"
          size="md"
          testid="org-disconnect"
          onClick={() => setConfirming(true)}
        >
          {t('Disconnect')}
        </Button>
      }
    >
      <div className="flex flex-col gap-3">
        <DeviceLine state={connection.deviceState} fingerprint={fingerprint} />
        <Devices url={connection.url} devices={devices} onChanged={reload} />
        <Lockboxes url={connection.url} lockboxes={lockboxes} onChanged={reload} />
        {failure && (
          <Note tone="bad" testid="org-disconnect-error">
            {failure}
          </Note>
        )}
      </div>
      {confirming && (
        <ConfirmSheet
          id={DISCONNECT_TITLE_ID}
          testid="org-disconnect-sheet"
          actionTestid="org-disconnect-confirm"
          title={t('Disconnect from {{tenant}}?', { tenant: connection.tenantName })}
          message={t(
            'The org lockboxes already on this device stay, but stop syncing. This device will need approving again if you reconnect.'
          )}
          action={t('Disconnect')}
          onConfirm={disconnect}
          onClose={() => setConfirming(false)}
        />
      )}
    </SettingsRow>
  )
}
