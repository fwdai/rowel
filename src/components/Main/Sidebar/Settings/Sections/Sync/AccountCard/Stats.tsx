import { useTranslation } from 'react-i18next'
import { useApp, selectWorkspaces } from '@/store'
import { LABEL } from '@/components/elements/tokens'

// The connected account in three figures, under the card's header.
export default function Stats({ lastSynced }: { lastSynced: string }) {
  const { t } = useTranslation()
  const workspaces = useApp(selectWorkspaces)

  const stats = [
    { label: t('Last synced'), value: lastSynced },
    { label: t('Lockboxes'), value: String(workspaces.length) },
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
