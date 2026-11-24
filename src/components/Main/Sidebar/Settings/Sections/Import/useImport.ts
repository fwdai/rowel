import { useCallback, useEffect, useState } from 'react'
import { claimOpenedFile, setEntries, stampSession, useApp } from '@/store'
import { importSwftx } from '@/api/vault'
import { importEntries, type ImportFormat, type ImportReport } from '@/api/imports'
import { syncNow } from '@/api/sync'
import { pickImportFile, pickImportBackup } from '@/api/pickers'
import type { PickedFile } from '@/api/tools'
import { describeError, errorKind } from '@/api/errors'
import { isBackupFile, isRestorableFile } from '@/lib/backup'
import { pickedPath } from '@/lib/picked'
import { t } from '@/i18n'
import { useSubpage } from '../../sectionNav'
import { useProgress } from './useProgress'

// Either a third-party export (parsed by the backend under `format`), a legacy
// `.swftx` vault, which is independently encrypted and needs its own password,
// or a `.rowel` backup — which is a whole sealed database and restores a fresh
// install rather than merging into this one, so it is only ever named here to
// say so. Each is the file as it was chosen: the path to read it by, and the
// name it is told by (see `@/lib/picked`).
export type Picked = (
  | { kind: 'format'; format: ImportFormat }
  | { kind: 'swftx' }
  | { kind: 'rowel' }
) &
  PickedFile

// A backup by its extension: ours is a `.rowel`, anything else the card or the
// OS hands over is taken for a legacy `.swftx` and asked for its password.
const pickedBackup = (file: PickedFile): Picked => ({
  kind: isBackupFile(file.name) ? 'rowel' : 'swftx',
  ...file
})

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
    // input; a `.rowel` is not imported at all.
    if (next.kind !== 'format') return
    setRunning(true)
    importEntries(next.path, next.format, true)
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
    start(pickedBackup(pickedPath(opened)))
  }, [opened, covered, start])

  // A pick that comes back after a lock is the old session's: dropped, like
  // everything else it held (see `stampSession`).
  const chooseFile = useCallback((format: ImportFormat) => {
    const current = stampSession()
    return pickImportFile().then(file => {
      if (file && current()) start({ kind: 'format', format, ...file })
    })
  }, [start])

  const chooseBackup = useCallback(() => {
    const current = stampSession()
    return pickImportBackup().then(file => {
      if (file && current()) start(pickedBackup(file))
    })
  }, [start])

  // A dropped or picked file with no tile behind it: one of our backups is
  // taken as the backup card would take it, anything else has its format
  // sniffed by the backend.
  const dropped = useCallback(
    (file: PickedFile) =>
      start(
        isRestorableFile(file.name)
          ? pickedBackup(file)
          : { kind: 'format', format: 'auto', ...file }
      ),
    [start]
  )

  const commit = () => {
    if (picked?.kind !== 'format' || running) return
    setRunning(true)
    setError(null)
    reset()
    importEntries(picked.path, picked.format, false)
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
    commit,
    runBackup
  }
}
