import { call } from './client'
import type { Attachment, AttachmentUsage } from './types'

/**
 * The largest file one attachment may be — Rust's `MAX_ATTACHMENT_BYTES`,
 * which is the cap that counts. Known here so a file the webview holds can be
 * refused before it is read and sent, not after.
 */
export const MAX_ATTACHMENT_BYTES = 10 * 1024 * 1024

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

/**
 * Attach a file the webview holds — what the phone's system picker handed its
 * file input, which has no path on disk. The bytes go over as the request's
 * raw body, the entry and the name as its headers (the name percent-encoded:
 * a header carries ASCII, a file is named in anything). No grant is involved:
 * nothing on disk is read for them. Rejects as `addAttachment` does.
 */
export const addAttachmentBytes = (
  entryId: string,
  name: string,
  bytes: Uint8Array
): Promise<Attachment> =>
  call('attachment_add_bytes', bytes, {
    headers: { 'x-entry-id': entryId, 'x-file-name': encodeURIComponent(name) }
  })

export const attachmentUsage = (): Promise<AttachmentUsage> => call('attachment_usage')

// Write an attachment to disk through the save dialog, under its own name. The
// bytes go from the vault to the file in Rust. Resolves to the chosen path, or
// null when the dialog was dismissed.
export const saveAttachment = (id: string): Promise<string | null> =>
  call('attachment_save', { id })

// The three commands below answer with bytes rather than JSON, which `invoke`
// hands over as an ArrayBuffer. (Each names its command literally:
// `contract.test` reads the names off these calls.)
const asBytes = (buffer: ArrayBuffer) => new Uint8Array(buffer)

// A small PNG of a picture, for its row in the list: bounded by the backend,
// so a list of images costs kilobytes rather than the files. Rejects
// `unsupported` for a picture the backend cannot decode (HEIC).
export const thumbnailAttachment = (id: string): Promise<Uint8Array<ArrayBuffer>> =>
  call<ArrayBuffer>('attachment_thumbnail', { id }).then(asBytes)

// One attachment's bytes, to show it.
export const readAttachment = (id: string): Promise<Uint8Array<ArrayBuffer>> =>
  call<ArrayBuffer>('attachment_read', { id }).then(asBytes)

// The same picture as a PNG, for an image the webview's own engine could not
// decode (a TIFF on Windows). Rejects `unsupported` for a file that is not a
// picture the backend can read either.
export const renderAttachment = (id: string): Promise<Uint8Array<ArrayBuffer>> =>
  call<ArrayBuffer>('attachment_render', { id }).then(asBytes)

// Remove an attachment for good.
export const deleteAttachment = (id: string): Promise<void> => call('attachment_delete', { id })
