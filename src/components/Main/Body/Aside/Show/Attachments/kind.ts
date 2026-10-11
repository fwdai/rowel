import type { Attachment } from '@/api/types'
import { isImagePath } from '@/lib/fileTypes'
import { humanSize } from '@/utils/size'

/** What can be made of a file in the webview, from the type guessed off its name. */
export type PreviewKind = 'image' | 'pdf' | 'text' | 'none'

const TEXT_EXTENSIONS = ['txt', 'md', 'csv', 'json', 'log']

/** The file's extension in capitals ("PDF"), or nothing for a name without one. */
export const extensionOf = (name: string): string | null => {
  const match = /\.([^.\\/]{1,8})$/.exec(name)
  return match ? match[1].toUpperCase() : null
}

// What the picker filters images by, as types — `jpg` and `tif` under the
// names their types go by.
const imageType = (extension: string) =>
  `image/${{ jpg: 'jpeg', tif: 'tiff' }[extension] ?? extension}`

/**
 * The file's type: the one the backend guessed when it was attached, or —
 * for a file attached before its format was in that table, which has none —
 * one guessed again here from the name: images by the same list the picker
 * filters by (`IMAGE_EXTENSIONS`), so what can be picked as a picture is what
 * is shown as one. Empty for a name that says nothing. This is also the type
 * the file's bytes are held under in the webview, which is what lets the
 * document viewer recognise a typeless PDF.
 */
export const typeOf = ({ mime, name }: Attachment): string => {
  if (mime) return mime
  const extension = extensionOf(name)?.toLowerCase() ?? ''
  if (isImagePath(name)) return imageType(extension)
  if (extension === 'pdf') return 'application/pdf'
  return TEXT_EXTENSIONS.includes(extension) ? 'text/plain' : ''
}

export const previewKind = (attachment: Attachment): PreviewKind => {
  const type = typeOf(attachment)
  if (type.startsWith('image/')) return 'image'
  if (type === 'application/pdf') return 'pdf'
  if (type.startsWith('text/') || type === 'application/json') return 'text'
  return 'none'
}

/** The line under a file's name: "670 KB · PNG". */
export const fileCaption = (attachment: Attachment): string => {
  const extension = extensionOf(attachment.name)
  const size = humanSize(attachment.size)
  return extension ? `${size} · ${extension}` : size
}
