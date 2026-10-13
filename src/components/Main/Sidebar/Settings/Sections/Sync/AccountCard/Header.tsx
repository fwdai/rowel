import { useTranslation } from 'react-i18next'
import { useApp } from '@/store'
import { providerName, type Provider } from '@/api/provider'
import { cx } from '@/utils/cx'
import { META } from '@/components/elements/tokens'
import ProviderMark from '@/components/elements/ProviderMark'
import { waitingFor } from '@/components/elements/providerCopy'
import { CloudGlyph } from '@/components/Main/icons'
import StatusPill from './StatusPill'
import Control from './Control'

interface Props {
  /** The account the card is about, or null before one is picked. */
  provider: Provider | null
  probing: boolean
  offered: boolean
  restoring: boolean
}

// The card's top row: whose account, where it stands, and the one control
// that moves it on.
export default function Header({ provider, probing, offered, restoring }: Props) {
  const { t } = useTranslation()
  const sync = useApp(state => state.sync)
  const name = provider && providerName(provider)

  const waiting = waitingFor(t, provider)
  // A connection the backend reports outranks a probe the press left pending:
  // the vault is this account's whatever the slice still says.
  const description = sync.pending
    ? waiting
    : sync.configured
      ? t('Connected')
      : probing
        ? waiting
        : offered
          ? t('This account already holds a vault')
          : t('Not connected')

  return (
    <div className="flex items-center gap-4 p-5">
      <div className="grid h-11 w-11 flex-none place-items-center rounded-lg bg-accent-soft text-accent">
        {provider ? <ProviderMark provider={provider} size={22} /> : <CloudGlyph size={20} />}
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <span
            data-testid="settings-account-name"
            className="text-md font-semibold tracking-display text-text"
          >
            {name ?? t('Cloud sync')}
          </span>
          <StatusPill sync={sync} />
        </div>
        <div className={cx('mt-0.5', META)}>{description}</div>
      </div>
      <div className="flex-none">
        <Control probing={probing} offered={offered} restoring={restoring} />
      </div>
    </div>
  )
}
