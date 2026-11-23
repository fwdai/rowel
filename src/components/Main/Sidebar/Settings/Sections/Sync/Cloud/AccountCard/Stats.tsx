import { useTranslation } from 'react-i18next'
import { inAccount } from '@/lib/workspace'
import { LABEL } from '@/components/elements/tokens'
import { useAccountLockboxes } from '../useAccountLockboxes'

// The connected account in three figures, under the card's header.
export default function Stats({ lastSynced }: { lastSynced: string }) {
  const { t } = useTranslation()

  // The lockboxes the account holds, as the list under the card shows them:
  // those here that live in its provider, by the reading Settings › Lockboxes
  // gives each (`workspaceProvider`), and those not on this device yet. For a
  // locked one that is as far as the registry goes: it knows the provider,
  // not whose account, so a lockbox in another account of the same provider
  // is counted here too.
  const held = useAccountLockboxes().filter(inAccount).length

  const stats = [
    { label: t('Last synced'), value: lastSynced },
    { label: t('Lockboxes'), value: String(held) },
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
