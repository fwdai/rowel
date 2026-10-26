import { useTranslation } from 'react-i18next'
import type { OrgDeviceState } from '@/api/org'
import Note from '@/components/elements/Note'

interface Props {
  state: OrgDeviceState | null
  /** This device's enrollment key, shown again while approval is waited for. */
  fingerprint: string | null
}

// Where this device stands with the server, in one sentence. A pending one
// names who it waits on and repeats the fingerprint they will compare, so
// the person approving from elsewhere can read it off this screen.
export default function DeviceLine({ state, fingerprint }: Props) {
  const { t } = useTranslation()

  if (state === 'approved')
    return (
      <Note tone="good" testid="org-device-line">
        {t('This device is enrolled')}
      </Note>
    )
  if (state === 'revoked')
    return (
      <Note tone="bad" testid="org-device-line">
        {t("This device's enrollment was revoked")}
      </Note>
    )
  if (state === 'pending')
    return (
      <Note testid="org-device-line">
        {t("Waiting for approval from another of your devices or an admin. They compare this device's key:")}{' '}
        <code className="font-mono text-text">{fingerprint ?? '…'}</code>
      </Note>
    )
  return <Note testid="org-device-line">{t('This device is not enrolled')}</Note>
}
