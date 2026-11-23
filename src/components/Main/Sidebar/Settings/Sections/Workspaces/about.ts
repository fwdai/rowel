import type { TFunction } from 'i18next'
import type { Workspace } from '@/api/types'
import { serverHost } from '@/api/org'
import { providerName, type Provider } from '@/api/provider'

interface Context {
  // The open workspace answers from its live session; a locked one only from
  // what the registry kept about it.
  current: boolean
  provider: Provider | null
  liveCount: number
}

/**
 * A workspace's size and where it lives, as one meta line ("284 items · Google
 * Drive"). The same facts the lock screen's picker says of each vault — the
 * open one's count is the live list rather than the registry's as-of-last-open
 * one. A locked vault never opened here since counts were kept has no count.
 * An org lockbox names the server it syncs through by host alone: the URL's
 * scheme and port are the app's business, not a line a person reads.
 */
export const workspaceAbout = (
  workspace: Workspace,
  { current, provider, liveCount }: Context,
  t: TFunction
): string => {
  const home = workspace.server
    ? t('Org lockbox · synced through {{host}}', { host: serverHost(workspace.server) })
    : provider
      ? providerName(provider)
      : t('This device')
  const count = current ? liveCount : workspace.itemCount
  return count === undefined ? home : `${t('{{count}} items', { count })} · ${home}`
}
