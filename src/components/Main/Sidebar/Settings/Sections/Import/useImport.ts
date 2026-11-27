import { useCallback, useEffect, useState } from 'react'
import { claimExchange, claimOpenedFile, setEntries, useApp } from '@/store'
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
// held by the backend (`exchange`), and the Apple Passwords tile on a device
// where that is how its data arrives, which has nothing to pick and can only
// say how (`exchangeGuide`).
export type Picked =
  | { kind: 'format'; format: ImportFormat; path: string }
  | { kind: 'swftx'; path: string }
  | { kind: 'rowel'; path: string }
  | { kind: 'exchange' }
  | { kind: 'exchangeGuide' }

// A backup by its extension: ours is a `.rowel`, anything else the card or the
// OS hands over is taken for a legacy `.swftx` and asked for its password.
const pickedBackup = (path: string): Picked => ({
  kind: isBackupFile(path) ? 'rowel' : 'swftx',
  path
})

// How a pick's rows are read, as a dry run or for real: a file is parsed under
// its format, a transfer is already in the backend's hands. Null for a pick
// with no rows to preview.
const reader = (picked: Picked): ((dryRun: boolean) => Promise<ImportReport>) | null =>
  picked.kind === 'format'
    ? dryRun => importEntries(picked.path, picked.format, dryRun)
    : picked.kind === 'exchange'
      ? importExchange
      : null

// One import flow for every tile: pick or drop a file, preview it (dry run),
// then commit. Preview and result both surface per-row errors.
export function useImport() {
  const [picked, setPicked] = useState<Picked | null>(null)
  const [preview, setPreview] = useState<ImportReport | null>(null)
  const [result, setResult] = useState<ImportReport | null>(null)
  const [count, setCount] = useState<number | null>(null)
  const [running, setRunning] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const { progress, reset } = useProgress()

  const start = useCallback((next: Picked) => {
    setPicked(next)
    setPreview(null)
    setResult(null)
    setCount(null)
    setError(null)
    // A backup cannot be inspected without its password, so it waits for
    // input; a `.rowel` is not imported at all, and neither is a guide.
    const read = reader(next)
    if (!read) return
    setRunning(true)
    read(true)
      .then(setPreview)
      .catch((e: unknown) => setError(describeError(e)))
      .finally(() => setRunning(false))
  }, [])

  // A backup the OS opened with the app, once this section is on screen,
  // taken as the backup card would take it. Under a sub-page it is not on
  // screen, though the phone keeps it mounted there: the file is left waiting,
  // which is what the page over it steps aside for (see `useSubpageState`).
  const opened = useApp(state => state.openedFile)
  const covered = useSubpage().subpage !== null
  useEffect(() => {
    if (!opened || covered) return
    claimOpenedFile()
    start(pickedBackup(opened))
  }, [opened, covered, start])

  // Another app's data, held by the backend: taken the same way, and on the
  // same terms.
  const exchange = useApp(state => state.exchangePending)
  useEffect(() => {
    if (!exchange || covered) return
    claimExchange()
    start({ kind: 'exchange' })
  }, [exchange, covered, start])

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
    (path: string) =>
      start(isRestorableFile(path) ? pickedBackup(path) : { kind: 'format', format: 'auto', path }),
    [start]
  )

  // The Apple Passwords tile where there is no export file to pick.
  const explainExchange = useCallback(() => start({ kind: 'exchangeGuide' }), [start])

  const commit = () => {
    const read = picked && reader(picked)
    if (!read || running) return
    setRunning(true)
    setError(null)
    reset()
    read(false)
      .then(report => {
        setEntries(report.entries)
        setResult(report)
        setPreview(null)
        // Publish the imported entries; a no-op when sync is not configured.
        if (report.imported > 0) syncNow().catch(() => {})
      })
      .catch((e: unknown) => setError(describeError(e)))
      .finally(() => setRunning(false))
  }

  const runBackup = (password: string) => {
    if (picked?.kind !== 'swftx' || running) return
    setRunning(true)
    setError(null)
    setCount(null)
    reset()
    importSwftx(picked.path, password)
      .then(({ count, entries }) => {
        setEntries(entries)
        setCount(count)
        // Publish the imported entries; a no-op when sync is not configured.
        syncNow().catch(() => {})
      })
      .catch((e: unknown) =>
        setError(
          errorKind(e) === 'invalidPassword' ? t('Invalid password for backup') : describeError(e)
        )
      )
      .finally(() => setRunning(false))
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
