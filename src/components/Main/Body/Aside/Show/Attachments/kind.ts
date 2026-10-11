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

/**
 * The backend guesses a type from the name when a file is attached and stores
 * it for good, so a file attached before its format was in that table has
 * none. For those the name is guessed from again here — images by the same
 * list the picker filters by (`IMAGE_EXTENSIONS`), so what can be picked as a
 * picture is what is shown as one.
 */
const typeOf = ({ mime, name }: Attachment): string => {
  if (mime) return mime
  if (isImagePath(name)) return 'image/'
  const extension = extensionOf(name)?.toLowerCase()
  if (extension === 'pdf') return 'application/pdf'
  return extension && TEXT_EXTENSIONS.includes(extension) ? 'text/' : ''
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
