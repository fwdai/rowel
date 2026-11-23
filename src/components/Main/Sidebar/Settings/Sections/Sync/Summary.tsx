import { useTranslation } from 'react-i18next'
import { useApp, openPasswordPrompt, selectWorkspaces } from '@/store'
import { syncProvider } from '@/api/sync'
import { providerName } from '@/api/provider'
import { backupCoverage } from '@/lib/workspace'
import { nearbyView, syncView } from '@/lib/syncView'
import { cx } from '@/utils/cx'
import Button from '@/components/elements/Button'
import ProviderMark from '@/components/elements/ProviderMark'
import { CARD, HERO_WASH, META } from '@/components/elements/tokens'
import { DiskGlyph, NearbyGlyph } from '@/components/Main/icons'
import { useSectionNav, useSubpage } from '../../sectionNav'

// Where the vault stands, above the ways to change it: the header chip's own
// verdict (`syncView`) spelled out, so the two can never disagree — on what is
// wrong, or on where it is put right. A vault that syncs nowhere is a state
// like any other, and is offered the one thing that then stands between a lost
// device and a lost vault — until every lockbox has a backup saved.
export default function Summary() {
  const { t } = useTranslation()
  const { open } = useSubpage()
  const go = useSectionNav()
  const sync = useApp(state => state.sync)
  const nearby = useApp(state => state.nearby)
  const backedUp = backupCoverage(useApp(selectWorkspaces)).oldest !== null
  const view = syncView(sync, nearby)
  const provider = syncProvider(sync)
  // Named beside the account only while it is doing its part: a device that
  // needs something done is its row's to say, not a way the vault is in sync.
  const nearbyTone = nearbyView(nearby)?.tone
  const nearbyOn = nearbyTone !== undefined && nearbyTone !== 'bad'

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

  // To where the failure's own controls are, which is not always a page of
  // this section (see `SyncFix`).
  const review = () => {
    if (view.fix === 'password') openPasswordPrompt()
    else if (view.fix === 'lockboxes') go('workspaces')
    else open({ key: view.fix === 'nearby' ? 'sync-nearby' : 'sync-cloud' })
  }

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
                onClick={review}
              >
                {t('Review')}
              </Button>
            )}
            {view.tone === 'local' && !backedUp && (
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
