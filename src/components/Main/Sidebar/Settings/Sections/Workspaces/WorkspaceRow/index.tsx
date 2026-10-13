import { useTranslation } from 'react-i18next'
import type { Workspace } from '@/api/types'
import { PRIMARY_WORKSPACE, workspaceLabel } from '@/lib/workspace'
import { cx } from '@/utils/cx'
import Button from '@/components/elements/Button'
import Chip from '@/components/elements/Chip'
import Monogram from '@/components/elements/Monogram'
import { META, ROW_HAIRLINE } from '@/components/elements/tokens'
import { useSubpage } from '../../../sectionNav'
import Menu from './Menu'

interface Props {
  workspace: Workspace
  // The open one: marked, and not offered as a place to switch to.
  current: boolean
  last: boolean
  // Its size and where it lives, already translated.
  about: string
  onSwitch: () => void
  onDelete: () => void
}

// One workspace on this device: its tile, its name over what is known of it,
// Switch when it is not the open one, and the rest behind ⋯.
export default function WorkspaceRow({
  workspace,
  current,
  last,
  about,
  onSwitch,
  onDelete
}: Props) {
  const { t } = useTranslation()
  const { open } = useSubpage()
  const label = workspaceLabel(workspace, t)

  return (
    <div data-testid={`workspace-row-${workspace.id}`} className={cx('px-4 py-3.5', ROW_HAIRLINE)}>
      <div className="flex items-center gap-3.5">
        <Monogram name={label} seed={workspace.id} color={workspace.color} size={36} />
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-center gap-2">
            <span className="truncate text-base font-medium text-text">{label}</span>
            {/* The primary is the workspace whose master password is the
                device's — the one a new workspace is sealed with and the one
                Touch ID enrols — and nothing else on the row says which that
                is once it has been renamed. */}
            {workspace.id === PRIMARY_WORKSPACE && (
              <Chip testid={`workspace-primary-${workspace.id}`}>{t('Primary')}</Chip>
            )}
            {current && (
              <Chip tone="good" dot>
                {t('Open now')}
              </Chip>
            )}
          </div>
          <div className={cx(META, 'mt-0.5 truncate')}>{about}</div>
        </div>
        <div className="flex flex-none items-center gap-1.5">
          {!current && (
            <Button
              variant="pale"
              size="md"
              testid={`workspace-switch-${workspace.id}`}
              onClick={onSwitch}
            >
              {t('Switch')}
            </Button>
          )}
          <Menu
            id={workspace.id}
            last={last}
            onEdit={() => open({ key: 'edit-workspace', id: workspace.id })}
            onDelete={onDelete}
          />
        </div>
      </div>
    </div>
  )
}
