import { useTranslation } from 'react-i18next'
import type { SyncStatus } from '@/api/sync'
import { cx } from '@/utils/cx'
import { META_TYPE } from '@/components/elements/tokens'

const TONES = {
  good: 'bg-good/10 text-good',
  busy: 'bg-accent-soft text-accent',
  bad: 'bg-bad/10 text-bad',
  idle: 'bg-tile text-text2'
} as const

// Where the connection stands, at a glance beside the card's title. A sync
// running outranks the last one's failure: it is the answer to it.
export default function StatusPill({ sync }: { sync: SyncStatus }) {
  const { t } = useTranslation()
  const [tone, label] = sync.inProgress
    ? (['busy', t('Syncing')] as const)
    : sync.error !== null
      ? (['bad', t('Last attempt failed')] as const)
      : sync.configured
        ? (['good', t('Up to date')] as const)
        : (['idle', t('Not connected')] as const)

  return (
    <span
      data-testid="settings-drive-status"
      data-tone={tone}
      className={cx(
        'flex h-5 items-center gap-1.5 rounded-full px-2 font-medium',
        META_TYPE,
        TONES[tone]
      )}
    >
      <span aria-hidden className="h-[5px] w-[5px] rounded-full bg-current" />
      {label}
    </span>
  )
}
