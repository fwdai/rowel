import { useTranslation } from 'react-i18next'
import type { Provider } from '@/api/provider'
import { workspaceLabel, type AccountLockbox } from '@/lib/workspace'
import { cx } from '@/utils/cx'
import Chip from '@/components/elements/Chip'
import Monogram from '@/components/elements/Monogram'
import { ROW_HAIRLINE } from '@/components/elements/tokens'
import Standing from './Standing'

interface Props {
  lockbox: Extract<AccountLockbox, { standing: 'both' | 'device' }>
  /** The account's provider. */
  provider: Provider
}

// A lockbox on this device: its tile and name as Settings › Lockboxes draws
// them, and where it stands with the account. Nothing to press. Switching is
// that page's; and switching to one that syncs nowhere would not bring it into
// the account, which turns away a connect from a lockbox it does not already
// hold (`sync_adopt_pending`).
export default function LocalRow({ lockbox: { workspace, standing, open }, provider }: Props) {
  const { t } = useTranslation()
  const label = workspaceLabel(workspace, t)
  const testid = `settings-cloud-lockbox-${workspace.id}`

  return (
    <div data-testid={testid} className={cx('flex items-center gap-3.5 px-4 py-3.5', ROW_HAIRLINE)}>
      <Monogram name={label} seed={workspace.id} color={workspace.color} size={32} />
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 items-center gap-2">
          <span className="truncate text-base font-medium text-text max-md:text-md">{label}</span>
          {open && (
            <Chip tone="good" dot>
              {t('Open now')}
            </Chip>
          )}
        </div>
        <div className="mt-0.5 text-sm text-text2 max-md:text-base">
          <Standing standing={standing} provider={provider} testid={`${testid}-status`} />
        </div>
      </div>
    </div>
  )
}
