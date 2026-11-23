import { useTranslation } from 'react-i18next'
import type { SyncStatus } from '@/api/sync'
import { cloudStanding, type SyncTone } from '@/lib/syncView'
import Chip, { type ChipTone } from '@/components/elements/Chip'

// Only a sync that landed is green: an account still waiting for its first
// one has put nothing anywhere yet.
const CHIP: Record<SyncTone, ChipTone> = {
  local: 'neutral',
  idle: 'neutral',
  loading: 'accent',
  bad: 'bad',
  good: 'good'
}

// Where the connection stands, at a glance beside the card's title: the
// account's standing (`cloudStanding`, as the overview's row says it).
export default function StatusPill({ sync }: { sync: SyncStatus }) {
  const { t } = useTranslation()
  const { tone, label } = cloudStanding(sync)

  return (
    <Chip tone={CHIP[tone]} dot testid="settings-drive-status">
      {t(label)}
    </Chip>
  )
}
