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
import { syncDeletedRemotely, syncErrorText, syncReupload } from '@/api/sync'
import Button from '@/components/elements/Button'
import Note from '@/components/elements/Note'
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

  // The account says *this* vault was deleted — from another device, or by
  // someone with access to the account — so its runs stop here. The copy here
  // is intact either way: the user can put it back in the account, or delete
  // it here too. The delete offered is the local one: the account is not asked
  // to remove anything more.
  const strandedHere = syncDeletedRemotely(sync)
  const reupload = () => syncReupload().catch((err: unknown) => setError(messageOf(err)))

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
          className="flex flex-wrap items-center justify-between gap-2 px-4 py-3 inset-shadow-hairline"
        >
          <Note tone="bad">{syncErrorText(sync)}</Note>
          <div className="flex flex-wrap gap-2">
            <Button
              variant="pale"
              size="md"
              testid="workspace-reupload-stranded"
              onClick={reupload}
            >
              {t('Re-upload it')}
            </Button>
            <Button
              variant="pale-danger"
              size="md"
              testid="workspace-delete-stranded"
              disabled={last}
              onClick={() => askDelete(active)}
            >
              {t('Delete it here too')}
            </Button>
          </div>
        </div>
      )}
      {last && (
        <Note testid="workspace-delete-last" className="px-4 py-3 inset-shadow-hairline">
          {t('Your only lockbox cannot be deleted. Add another one first.')}
        </Note>
      )}
      {error && (
        <Note tone="bad" testid="workspace-switch-error" className="px-4 py-3 inset-shadow-hairline">
          {error}
        </Note>
      )}
    </>
  )
}
