import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useApp, forgetDrive, setupDriveFailed, setupDriveReset } from '@/store'
import { syncAdoptPending, syncErrorText } from '@/api/sync'
import { describeError, errorKind } from '@/api/errors'
import Note from '@/components/elements/Note'
import SettingsGroup from '@/components/elements/SettingsGroup'
import DriveCard from './DriveCard'
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
  const drive = useApp(state => state.setupDrive)
  // The last failure as copy: said from the catalogue where Rust named a kind
  // the frontend has words for, and in Rust's own words otherwise.
  const syncFailure = syncErrorText(sync)

  // Leaving the section forgets a probe's pending account, as Settings ›
  // Workspaces does: a sign-in the user walked away from is never adopted later.
  // Only when there is one to forget — this is the section Settings opens on,
  // so every visit to another section would otherwise cost a backend call.
  useEffect(
    () => () => {
      if (useApp.getState().setupDrive.status !== 'idle') forgetDrive()
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
    if (drive.status !== 'empty' && drive.status !== 'found') {
      // A restore in flight keeps the offer it was made from; a restore that
      // failed comes back as 'found' and finds the offer still standing.
      if (drive.status !== 'restoring') {
        asked.current = false
        setRefused(false)
      }
      return
    }
    if (asked.current) return
    asked.current = true
    syncAdoptPending().then(setupDriveReset, (error: unknown) => {
      if (errorKind(error) === 'vaultNotInAccount') setRefused(true)
      else setupDriveFailed(describeError(error))
    })
  }, [drive.status])

  const deciding = (drive.status === 'empty' || drive.status === 'found') && !refused
  const probing = drive.status === 'pending' || deciding
  const offered = refused && (drive.status === 'found' || drive.status === 'restoring')
  const restoring = drive.status === 'restoring'

  return (
    <>
      <section className="mb-7">
        <DriveCard probing={probing} offered={offered} restoring={restoring} />
        {syncFailure && <ErrorNote message={syncFailure} />}
        {drive.status === 'error' && (
          <ErrorNote message={drive.error || t('Something went wrong')} />
        )}
      </section>

      {sync.configured && (
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
