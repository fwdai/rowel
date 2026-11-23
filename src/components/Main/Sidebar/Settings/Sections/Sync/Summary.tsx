import { useTranslation } from 'react-i18next'
import { useApp } from '@/store'
import { syncProvider } from '@/api/sync'
import { providerName } from '@/api/provider'
import { cx } from '@/utils/cx'
import Button from '@/components/elements/Button'
import ProviderMark from '@/components/elements/ProviderMark'
import { CARD, HERO_WASH, META } from '@/components/elements/tokens'
import { DiskGlyph, NearbyGlyph } from '@/components/Main/icons'
import { syncView } from '@/components/Main/Header/SyncIndicator/state'
import { useSubpage } from '../../sectionNav'

// Where the vault stands, above the ways to change it: the header chip's own
// verdict (`syncView`) spelled out, so the two can never disagree. A vault
// that syncs nowhere is a state like any other, and is offered the one thing
// that then stands between a lost device and a lost vault.
export default function Summary() {
  const { t } = useTranslation()
  const { open } = useSubpage()
  const sync = useApp(state => state.sync)
  const nearby = useApp(state => state.nearby)
  const view = syncView(sync, nearby)
  const provider = syncProvider(sync)
  const nearbyOn = !!nearby?.available && nearby.enabled

  const title =
    view.tone === 'good'
      ? t('Your vault is in sync')
      : view.tone === 'local'
        ? t('On this device only')
        : t(view.message)

  const through = provider && { provider: providerName(provider) }
  const description =
    view.tone === 'bad'
      ? view.detail
      : view.tone === 'local'
        ? t(
            'Nothing leaves this device, and {{appName}} works fully that way. Keep a backup so a lost device is not a lost vault.'
          )
        : through
          ? nearbyOn
            ? t('Through {{provider}} and nearby devices', through)
            : t('Through {{provider}}', through)
          : t(
              'Your devices on the same network sync directly while {{appName}} is open and unlocked on both. Nothing leaves your network.'
            )

  return (
    <section className="mb-7">
      <div
        data-testid="settings-sync-summary"
        data-tone={view.tone}
        className={cx(CARD, HERO_WASH)}
      >
        <div className="flex items-center gap-4 p-5">
          <div className="grid h-11 w-11 flex-none place-items-center rounded-lg bg-accent-soft text-accent">
            {provider ? (
              <ProviderMark provider={provider} size={22} />
            ) : view.tone === 'local' ? (
              <DiskGlyph size={20} />
            ) : (
              <NearbyGlyph size={20} />
            )}
          </div>
          <div className="flex min-w-0 flex-1 flex-wrap items-center gap-x-3.5 gap-y-2.5">
            <div className="min-w-0 flex-1 basis-40">
              <div className="text-md font-semibold tracking-display text-text">{title}</div>
              {description && <div className={cx('mt-0.5', META)}>{description}</div>}
            </div>
            {view.tone === 'bad' && (
              <Button
                variant="pale"
                size="md"
                className="flex-none"
                testid="settings-sync-review"
                onClick={() => open({ key: provider ? 'sync-cloud' : 'sync-nearby' })}
              >
                {t('Review')}
              </Button>
            )}
            {view.tone === 'local' && (
              <Button
                variant="pale"
                size="md"
                className="flex-none"
                testid="settings-sync-backup"
                onClick={() => open({ key: 'sync-backup' })}
              >
                {t('Save a backup')}
              </Button>
            )}
          </div>
        </div>
      </div>
    </section>
  )
}
