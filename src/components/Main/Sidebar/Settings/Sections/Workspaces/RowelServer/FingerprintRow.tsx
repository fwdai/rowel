import { useTranslation } from 'react-i18next'
import SettingsRow from '@/components/elements/SettingsRow'
import { FingerprintGlyph } from '@/components/Main/icons'

// This device's enrollment key, as the fingerprint a person reads out. It is
// what another of the member's devices, or an admin, compares before approving
// this device — so it is on screen before any server is, and stays there.
export default function FingerprintRow({ fingerprint }: { fingerprint: string | null }) {
  const { t } = useTranslation()
  return (
    <SettingsRow
      label={t("This device's enrollment key")}
      icon={<FingerprintGlyph />}
      description={t(
        'Another of your devices, or an admin, compares this fingerprint before approving this device on a server.'
      )}
      testid="org-fingerprint-row"
      control={
        <code data-testid="org-fingerprint" className="font-mono text-sm text-text">
          {fingerprint ?? '…'}
        </code>
      }
    />
  )
}
