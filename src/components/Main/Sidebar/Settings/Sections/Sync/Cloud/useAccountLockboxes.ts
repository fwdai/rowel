import { useApp, selectActiveWorkspace, selectWorkspaces } from '@/store'
import { accountLockboxes, type AccountLockbox } from '@/lib/workspace'

// The connected account's lockboxes (`accountLockboxes`), read once for the
// card's figure and the list under it alike.
export function useAccountLockboxes(): AccountLockbox[] {
  const workspaces = useApp(selectWorkspaces)
  const active = useApp(selectActiveWorkspace)
  const sync = useApp(state => state.sync)
  const remote = useApp(state => state.remoteVaults)
  return accountLockboxes(workspaces, remote, { active, sync })
}
