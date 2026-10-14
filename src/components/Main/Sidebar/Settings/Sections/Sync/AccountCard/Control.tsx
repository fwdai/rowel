import { useTranslation } from 'react-i18next'
import { useApp, forgetAccount } from '@/store'
import { syncNow } from '@/api/sync'
import Button from '@/components/elements/Button'
import { RefreshGlyph } from '../../../../../icons'

interface Props {
  probing: boolean
  offered: boolean
  restoring: boolean
}

// The header's one control: Sync now once connected, a spinner while a connect
// is out — or, with the account's vaults on offer, the way back out of it.
// Before any of that the card offers one connect per provider instead.
export default function Control({ probing, offered, restoring }: Props) {
  const { t } = useTranslation()
  const sync = useApp(state => state.sync)

  if (sync.configured)
    return (
      <Button
        variant="pale"
        size="md"
        loading={sync.inProgress}
        onClick={() => syncNow()}
        testid="settings-sync-now"
      >
        <RefreshGlyph />
        {t('Sync now')}
      </Button>
    )

  // Withdrawn while the restore runs: the backend holds the account for its
  // length and would refuse the disconnect this is.
  if (offered)
    return restoring ? null : (
      <Button variant="pale" size="md" onClick={forgetAccount} testid="settings-drive-cancel">
        {t('Cancel')}
      </Button>
    )

  // Consent happens in the browser, and the backend reports every step of it
  // through the `setup:account:*` events — the browser opening, the answer, a
  // failure — so this only waits on them.
  if (sync.pending || sync.inProgress || probing)
    return (
      <Button variant="pale" size="md" loading testid="settings-account-connecting">
        {t('Connect')}
      </Button>
    )

  return null
}
