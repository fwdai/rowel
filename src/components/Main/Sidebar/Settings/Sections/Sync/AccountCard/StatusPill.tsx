import { useTranslation } from 'react-i18next'
import type { SyncStatus } from '@/api/sync'
import Chip, { type ChipTone } from '@/components/elements/Chip'

// Where the connection stands, at a glance beside the card's title. A sync
// running outranks the last one's failure: it is the answer to it.
export default function StatusPill({ sync }: { sync: SyncStatus }) {
  const { t } = useTranslation()
  const [tone, label]: [ChipTone, string] = sync.inProgress
    ? ['accent', t('Syncing')]
    : sync.error !== null
      ? ['bad', t('Last attempt failed')]
      : sync.configured
        ? ['good', t('Up to date')]
        : ['neutral', t('Not connected')]

  return (
    <Chip tone={tone} dot testid="settings-drive-status">
      {label}
    </Chip>
  )
}
