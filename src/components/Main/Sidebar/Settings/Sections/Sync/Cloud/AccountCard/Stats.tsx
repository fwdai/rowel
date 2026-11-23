import { useTranslation } from 'react-i18next'
import { useApp, selectActiveWorkspace, selectWorkspaces } from '@/store'
import { syncProvider } from '@/api/sync'
import { workspaceProvider } from '@/lib/workspace'
import { LABEL } from '@/components/elements/tokens'

// The connected account in three figures, under the card's header.
export default function Stats({ lastSynced }: { lastSynced: string }) {
  const { t } = useTranslation()
  const workspaces = useApp(selectWorkspaces)
  const active = useApp(selectActiveWorkspace)
  const sync = useApp(state => state.sync)

  // The lockboxes that live in this account's provider, by the same reading
  // Settings › Lockboxes gives each of them (`workspaceProvider`). For a
  // locked one that is as far as the registry goes: it knows the provider,
  // not whose account, so a lockbox in another account of the same provider
  // is counted here too.
  const provider = syncProvider(sync)
  const inAccount = workspaces.filter(
    workspace => workspaceProvider(workspace, { active, sync }) === provider
  ).length

  const stats = [
    { label: t('Last synced'), value: lastSynced },
    { label: t('Lockboxes'), value: String(inAccount) },
    { label: t('Encryption'), value: t('End-to-end') }
  ]

  return (
    <div className="grid grid-cols-3 border-t border-line">
      {stats.map(stat => (
        <div
          key={stat.label}
          className="min-w-0 px-5 py-3.5 not-first:border-l not-first:border-line"
        >
          <div className={LABEL}>{stat.label}</div>
          <div className="mt-0.5 truncate text-base font-medium text-text">{stat.value}</div>
        </div>
      ))}
    </div>
  )
}
