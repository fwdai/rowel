import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useApp, forgetAccount, setupAccountFailed, setupAccountReset } from '@/store'
import { syncAdoptPending, syncErrorText, syncProvider } from '@/api/sync'
import { providerShares } from '@/api/provider'
import { describeError, errorKind } from '@/api/errors'
import Note from '@/components/elements/Note'
import SettingsGroup from '@/components/elements/SettingsGroup'
import AccountCard from './AccountCard'
import BackupRow from './BackupRow'
import SharesRow from './SharesRow'

// Under the card, at its own inset.
const ErrorNote = ({ message }: { message: string }) => (
  <Note tone="bad" testid="settings-sync-error" className="px-5 pt-2.5">
    {message}
  </Note>
)

export default function Sync() {
  const { t } = useTranslation()
  const sync = useApp(state => state.sync)
  // A connect answers here rather than on `sync` (see `syncConnect`): the
  // account is probed first, and whether this vault may join it is the
  // backend's to say (`syncAdoptPending`).
  const account = useApp(state => state.setupAccount)
  // The last failure as copy: said from the catalogue where Rust named a kind
  // the frontend has words for, and in Rust's own words otherwise.
  const syncFailure = syncErrorText(sync)

  // Leaving the section forgets a probe's pending account, as Settings ›
  // Workspaces does: a sign-in the user walked away from is never adopted later.
  // Only when there is one to forget — this is the section Settings opens on,
  // so every visit to another section would otherwise cost a backend call.
  useEffect(
    () => () => {
      if (useApp.getState().setupAccount.status !== 'idle') forgetAccount()
    },
    []
  )

  // Every probe is put to the backend once: it adopts the account (empty, or
  // already holding this vault) or refuses it, and only a refusal shows the
  // account's vaults to restore. Once per probe — the status outlives the
  // call, and a second adopt would find the tokens already taken.
  const asked = useRef(false)
  const [refused, setRefused] = useState(false)
  useEffect(() => {
    if (account.status !== 'empty' && account.status !== 'found') {
      // A restore in flight keeps the offer it was made from; a restore that
      // failed comes back as 'found' and finds the offer still standing.
      if (account.status !== 'restoring') {
        asked.current = false
        setRefused(false)
      }
      return
    }
    if (asked.current) return
    asked.current = true
    syncAdoptPending().then(setupAccountReset, (error: unknown) => {
      if (errorKind(error) === 'vaultNotInAccount') setRefused(true)
      else setupAccountFailed(describeError(error))
    })
  }, [account.status])

  const deciding = (account.status === 'empty' || account.status === 'found') && !refused
  const probing = account.status === 'pending' || deciding
  const offered = refused && (account.status === 'found' || account.status === 'restoring')
  const restoring = account.status === 'restoring'

  return (
    <>
      <section className="mb-7">
        <AccountCard probing={probing} offered={offered} restoring={restoring} />
        {syncFailure && <ErrorNote message={syncFailure} />}
        {account.status === 'error' && (
          <ErrorNote message={account.error || t('Something went wrong')} />
        )}
      </section>

      {/* Shares are Drive files: another provider has nowhere to put one yet. */}
      {providerShares(syncProvider(sync)) && (
        <SettingsGroup label={t('Sharing')}>
          <SharesRow />
        </SettingsGroup>
      )}

      <SettingsGroup label={t('Offline backup')}>
        <BackupRow />
      </SettingsGroup>
    </>
  )
}
