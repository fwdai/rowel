import { useTranslation } from 'react-i18next'
import type { SyncStatus } from '@/api/sync'
import { cloudView } from '@/lib/syncView'
import Chip, { type ChipTone } from '@/components/elements/Chip'

// Where the connection stands, at a glance beside the card's title: the
// account's standing (`cloudView`, as the chip and the overview's row read
// it) in the pill's words.
export default function StatusPill({ sync }: { sync: SyncStatus }) {
  const { t } = useTranslation()
  const tone = cloudView(sync)?.tone
  const [chip, label]: [ChipTone, string] =
    tone === 'loading'
      ? ['accent', t('Syncing')]
      : tone === 'bad'
        ? ['bad', t('Last attempt failed')]
        : tone
          ? ['good', t('Up to date')]
          : ['neutral', t('Not connected')]

  return (
    <Chip tone={chip} dot testid="settings-drive-status">
      {label}
    </Chip>
  )
}
