import { useCallback, useEffect, useRef, useState } from 'react'
import {
  claimOpenedFile,
  exchangeShown,
  setEntries,
  syncExchange,
  useApp
} from '@/store'
import { importSwftx } from '@/api/vault'
import {
  importEntries,
  importExchange,
  type ImportFormat,
  type ImportReport
} from '@/api/imports'
import { syncNow } from '@/api/sync'
import { pickImportFile, pickImportBackup } from '@/services/pickers'
import { describeError, errorKind } from '@/api/errors'
import { isBackupFile, isRestorableFile } from '@/lib/backup'
import { t } from '@/i18n'
import { useSubpage } from '../../sectionNav'
import { useProgress } from './useProgress'

// Either a third-party export (parsed by the backend under `format`), a legacy
// `.swftx` vault, which is independently encrypted and needs its own password,
// or a `.rowel` backup — which is a whole sealed database and restores a fresh
// install rather than merging into this one, so it is only ever named here to
// say so.
//
// Or no file at all: another app's data, handed over through the system and
// held by the backend under its arrival number (`exchange`), and the Apple
// Passwords tile on a device where that is how its data arrives, which has
// nothing to pick and can only say how (`exchangeGuide`).
export type Picked =
  | { kind: 'format'; format: ImportFormat; path: string }
  | { kind: 'swftx'; path: string }
  | { kind: 'rowel'; path: string }
  | { kind: 'exchange'; arrival: number }
  | { kind: 'exchangeGuide' }

// A backup by its extension: ours is a `.rowel`, anything else the card or the
// OS hands over is taken for a legacy `.swftx` and asked for its password.
const pickedBackup = (path: string): Picked => ({
  kind: isBackupFile(path) ? 'rowel' : 'swftx',
  path
})

// How a pick's rows are read, as a dry run or for real: a file is parsed under
// its format, a transfer is read by its number from the backend that holds it
// — and either read may spend it (an import, a failure told once), so the
// store's copy of what is held is refreshed after. Null for a pick with no
// rows to preview.
const reader = (picked: Picked): ((dryRun: boolean) => Promise<ImportReport>) | null =>
  picked.kind === 'format'
    ? dryRun => importEntries(picked.path, picked.format, dryRun)
    : picked.kind === 'exchange'
      ? dryRun => importExchange(picked.arrival, dryRun).finally(syncExchange)
      : null

// One import flow for every tile: pick or drop a file, preview it (dry run),
// then commit. Preview and result both surface per-row errors.
//
// One thing runs at a time. A preview or an import under way owns the card
// until it ends, and nothing else starts meanwhile: the tiles are disabled,
// and what arrives from outside (a file the OS opens, another app's transfer)
// stays in the store, where it was put, until the flow is idle. So an answer
// never lands on a card that has moved on, and none has to be told apart from
// a newer one.
export function useImport() {
  const [picked, setPicked] = useState<Picked | null>(null)
  const [preview, setPreview] = useState<ImportReport | null>(null)
  const [result, setResult] = useState<ImportReport | null>(null)
  const [count, setCount] = useState<number | null>(null)
  const [running, setRunning] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const { progress, reset } = useProgress()

  // `running` as the next call sees it, not as the last render did. `work`
  // comes with what it does with its answer already attached, so the card is
  // filled before it is free again.
  const busy = useRef(false)
  const run = useCallback((work: Promise<unknown>) => {
    busy.current = true
    setRunning(true)
    void work.finally(() => {
      busy.current = false
      setRunning(false)
    })
  }, [])

  // Put `next` on the card, and preview it if it has rows to show. Refused
  // while something is running; the caller keeps what it offered.
  const start = useCallback(
    (next: Picked): boolean => {
      if (busy.current) return false
      setPicked(next)
      setPreview(null)
      setResult(null)
      setCount(null)
      setError(null)
      // A backup cannot be inspected without its password, so it waits for
      // input; a `.rowel` is not imported at all, and neither is a guide.
      const read = reader(next)
      if (read)
        run(
          read(true)
            .then(setPreview)
            .catch((e: unknown) => setError(describeError(e)))
        )
      return true
    },
    [run]
  )

  // A backup the OS opened with the app, once this section is on screen and
  // idle, taken as the backup card would take it. Under a sub-page it is not
  // on screen, though the phone keeps it mounted there: the file is left
  // waiting, which is what the page over it steps aside for (see
  // `useSubpageState`).
  const opened = useApp(state => state.openedFile)
  const covered = useSubpage().subpage !== null
  useEffect(() => {
    if (!opened || covered || running) return
    if (start(pickedBackup(opened))) claimOpenedFile()
  }, [opened, covered, running, start])

  // Another app's transfer, on the same terms. The backend holds it until it
  // is imported, so it is this section's to show every time the section comes
  // on screen, not only the first: `offered` is the arrival this visit has
  // already put on the card, which keeps it from taking the card back from
  // something the user picked since.
  const exchange = useApp(state => state.exchange)
  const offered = useRef<number | null>(null)
  useEffect(() => {
    if (!exchange || covered || running || offered.current === exchange.arrival) return
    if (!start({ kind: 'exchange', arrival: exchange.arrival })) return
    offered.current = exchange.arrival
    exchangeShown(exchange.arrival)
  }, [exchange, covered, running, start])

  const chooseFile = useCallback(
    (format: ImportFormat) =>
      pickImportFile().then(path => {
        if (path) start({ kind: 'format', format, path })
      }),
    [start]
  )

  const chooseBackup = useCallback(
    () =>
      pickImportBackup().then(path => {
        if (path) start(pickedBackup(path))
      }),
    [start]
  )

  // A dropped file has no tile behind it: one of our backups is taken as the
  // backup card would take it, anything else has its format sniffed by the
  // backend.
  const dropped = useCallback(
    (path: string) => {
      start(isRestorableFile(path) ? pickedBackup(path) : { kind: 'format', format: 'auto', path })
    },
    [start]
  )

  // The Apple Passwords tile where there is no export file to pick.
  const explainExchange = useCallback(() => {
    start({ kind: 'exchangeGuide' })
  }, [start])

  const commit = () => {
    const read = picked && reader(picked)
    if (!read || busy.current) return
    setError(null)
    reset()
    run(
      read(false)
        .then(report => {
          setEntries(report.entries)
          setResult(report)
          setPreview(null)
          // Publish the imported entries; a no-op when sync is not configured.
          if (report.imported > 0) syncNow().catch(() => {})
        })
        .catch((e: unknown) => setError(describeError(e)))
    )
  }

  const runBackup = (password: string) => {
    if (picked?.kind !== 'swftx' || busy.current) return
    setError(null)
    setCount(null)
    reset()
    run(
      importSwftx(picked.path, password)
        .then(({ count, entries }) => {
          setEntries(entries)
          setCount(count)
          // Publish the imported entries; a no-op when sync is not configured.
          syncNow().catch(() => {})
        })
        .catch((e: unknown) =>
          setError(
            errorKind(e) === 'invalidPassword'
              ? t('Invalid password for backup')
              : describeError(e)
          )
        )
    )
  }

  return {
    picked,
    preview,
    result,
    count,
    running,
    error,
    progress,
    chooseFile,
    chooseBackup,
    dropped,
    explainExchange,
    commit,
    runBackup
  }
}
