import { useTranslation } from 'react-i18next'
import { useApp } from '@/store'
import { syncErrorText, syncProvider } from '@/api/sync'
import { providerName } from '@/api/provider'
import { useDates } from '@/hooks/useDates'
import { cx } from '@/utils/cx'
import { DiskGlyph } from '@/components/Main/icons'
import ProviderMark from './ProviderMark'
import Tooltip from './Tooltip'

interface Props {
  /** Which way the tooltip opens; `top` for an indicator in bottom chrome. */
  side?: 'top' | 'bottom'
}

// Where the vault lives, as a glyph with a status dot: Drive's own mark when
// it syncs (green settled, amber while a run is out, red after a failure),
// a disk when it is kept on this device only. The sentence lives in the
// tooltip and in the accessible name, so the chrome stays a glyph wide.
export default function SyncIndicator({ side = 'bottom' }: Props) {
  const { t } = useTranslation()
  const { relativeLong } = useDates()
  const sync = useApp(state => state.sync)

  const provider = syncProvider(sync)
  const name = provider && providerName(provider)
  const failed = sync.configured && sync.error !== null
  const title = !name
    ? t('Stored on this device')
    : sync.inProgress
      ? t('Syncing…')
      : failed
        ? t('{{provider}} sync failed', { provider: name })
        : t('Synced with {{provider}}', { provider: name })
  const detail = !sync.configured
    ? null
    : failed
      ? syncErrorText(sync)
      : sync.lastSyncedAt
        ? t('Last synced {{when}}', { when: relativeLong(sync.lastSyncedAt) })
        : null

  return (
    <Tooltip
      side={side}
      content={
        <span className="flex flex-col items-center gap-px">
          <span>{title}</span>
          {detail && <span className="text-xs opacity-70">{detail}</span>}
        </span>
      }
    >
      <span
        role="img"
        aria-label={detail ? `${title}. ${detail}` : title}
        data-testid="sync-indicator"
        data-state={!sync.configured ? 'off' : failed ? 'error' : sync.inProgress ? 'busy' : 'on'}
        className="relative grid h-7 w-7 place-items-center rounded-sm text-text3 transition-colors hover:bg-hover hover:text-text2"
      >
        {provider ? (
          <>
            <ProviderMark provider={provider} size={15} />
            <span
              aria-hidden
              className={cx(
                'absolute right-1 bottom-1 h-[7px] w-[7px] rounded-full ring-[1.5px] ring-app',
                failed ? 'bg-bad' : sync.inProgress ? 'animate-pulse bg-warn' : 'bg-good'
              )}
            />
          </>
        ) : (
          <DiskGlyph size={15} />
        )}
      </span>
    </Tooltip>
  )
}
