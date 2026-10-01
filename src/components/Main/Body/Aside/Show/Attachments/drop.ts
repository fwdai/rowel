import type { EntryMeta, EntryType } from '@/api/types'
import { isModalOpen, selectCurrent, useVault } from '@/store'
import { isImagePath } from '@/lib/fileTypes'
import { fileNameOf, isEnvFileName } from '@/kinds/env/ingest'

/** Every kind takes files but `env`, whose file is the entry itself. */
export const takesAttachments = (type: EntryType): boolean => type !== 'env'

/**
 * The entry a file dropped on the window now would be attached to: the one
 * open for reading — live, of a kind that takes files, no editor and no dialog
 * up. Null when a drop is somebody else's business.
 */
export const attachTarget = (): EntryMeta | null => {
  const state = useVault.getState()
  const current = selectCurrent(state)
  if (!current || current.deletedAt || state.editing || state.creating) return null
  if (!takesAttachments(current.type) || isModalOpen()) return null
  return current
}

/**
 * Whether a dropped path is the attachments' to take while an entry is open.
 * Drops are routed by kind and that stays so: an image is the scanner's and a
 * file named like an env is the env flow's (`EnvDrop`), wherever they land.
 * Everything else, dropped on an open entry, is attached to it — and the env
 * flow leaves it unread, so the drop's grant is still there to spend.
 */
export const isAttachDrop = (path: string): boolean =>
  !isImagePath(path) && !isEnvFileName(fileNameOf(path))
