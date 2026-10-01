import { call } from './client'
import type { Attachment, AttachmentUsage } from './types'

// The live attachments of one entry, oldest first: names and sizes only.
export const listAttachments = (entryId: string): Promise<Attachment[]> =>
  call('attachment_list', { entryId })

/**
 * Attach the file at `path` to an entry. The path has to be one the backend
 * saw the user choose — `pickFileToRead('attachment')`, or a drop on the
 * window — and the file is read and sealed there; rejects `fileTooLarge` past
 * the per-file cap and `vaultFull` past what is left of the vault's budget.
 */
export const addAttachment = (entryId: string, path: string): Promise<Attachment> =>
  call('attachment_add', { entryId, path })

// Write an attachment to disk through the save dialog, under its own name. The
// bytes go from the vault to the file in Rust. Resolves to the chosen path, or
// null when the dialog was dismissed.
export const attachmentUsage = (): Promise<AttachmentUsage> => call('attachment_usage')

export const saveAttachment = (id: string): Promise<string | null> =>
  call('attachment_save', { id })

// Remove an attachment for good.
export const deleteAttachment = (id: string): Promise<void> => call('attachment_delete', { id })
