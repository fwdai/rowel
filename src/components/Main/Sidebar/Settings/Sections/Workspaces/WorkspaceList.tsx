import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import {
  useApp,
  useVault,
  selectWorkspaces,
  selectActiveWorkspace,
  switchWorkspace
} from '@/store'
import { messageOf } from '@/api/errors'
import type { Workspace } from '@/api/types'
import { syncDeletedRemotely, syncErrorText } from '@/api/sync'
import Button from '@/components/elements/Button'
import { useSubpage } from '../../sectionNav'
import WorkspaceRow from './WorkspaceRow'
import { workspaceAbout, workspaceProvider } from './about'

export default function WorkspaceList() {
  const { t } = useTranslation()
  const list = useApp(selectWorkspaces)
  const active = useApp(selectActiveWorkspace)
  const sync = useApp(state => state.sync)
  const liveCount = useVault(state => state.items.length)
  // The one way a switch is refused: a sync flow is mid-flight in this
  // workspace, and moving the paths under it would land its files elsewhere.
  const [error, setError] = useState<string | null>(null)
  const { open } = useSubpage()
  // The confirmation is a page under this section, like Edit.
  const askDelete = (id: string) => open({ key: 'delete-workspace', id })

  // A device always has a vault to open, so the last workspace stays. Every
  // other one may go, the open one included: deleting it ends its session and
  // the app comes back on a survivor's lock screen.
  const last = list.length < 2

  // The account's copy of *this* vault is gone — deleted from another device —
  // so its runs stop here until this workspace goes too. The delete offered for
  // it is the local one: there is nothing left in the account to remove.
  const strandedHere = syncDeletedRemotely(sync)

  // Where its copy lives, if it has one to delete along with it.
  const provider = (workspace: Workspace) => workspaceProvider(workspace, { active, sync })

  return (
    <>
      {list.map(workspace => {
        const current = workspace.id === active
        return (
          <WorkspaceRow
            key={workspace.id}
            workspace={workspace}
            current={current}
            last={last}
            about={workspaceAbout(
              workspace,
              { current, provider: provider(workspace), liveCount },
              t
            )}
            onSwitch={() =>
              switchWorkspace(workspace.id).catch((err: unknown) => setError(messageOf(err)))
            }
            onDelete={() => askDelete(workspace.id)}
          />
        )
      })}
      {strandedHere && (
        <div
          data-testid="workspace-deleted-remotely"
          className="flex flex-wrap items-center justify-between gap-2 px-4 py-3 text-sm text-text2 inset-shadow-hairline"
        >
          <span className="text-bad">{syncErrorText(sync)}</span>
          <Button
            variant="pale"
            size="md"
            className="text-bad hover:text-bad"
            testid="workspace-delete-stranded"
            disabled={last}
            onClick={() => askDelete(active)}
          >
            {t('Delete it here too')}
          </Button>
        </div>
      )}
      {last && (
        <div
          data-testid="workspace-delete-last"
          className="px-4 py-3 text-sm text-text2 inset-shadow-hairline"
        >
          {t('Your only workspace cannot be deleted. Add another one first.')}
        </div>
      )}
      {error && (
        <div
          data-testid="workspace-switch-error"
          className="px-4 py-3 text-sm text-bad inset-shadow-hairline"
        >
          {error}
        </div>
      )}
    </>
  )
}
