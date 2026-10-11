import type { ChangeEvent, Ref } from 'react'

interface Props {
  testid: string
  /** The input's `accept`: `image/*` for a photo, left out for any file. */
  accept?: string
  /** Open the camera outright rather than the system's source sheet. */
  capture?: boolean
  /** What the user chose. Not called when the picker is dismissed. */
  onFile: (file: File) => void
  /** The input itself, for the caller to `click()` — nothing else opens it. */
  ref: Ref<HTMLInputElement>
}

/**
 * The webview's own file input, for what only it can reach: the phone's
 * camera. With `capture`, iOS opens the camera outright on `click()`, and what
 * it takes comes back as a `File` on no path the backend could be pointed at,
 * so it travels as bytes (`scan_image_bytes`, `attachment_add_bytes`). The
 * camera's permission prompt is the system's (the usage string is in the iOS
 * `Info.plist`).
 *
 * Without `capture` iOS answers the click with its own source sheet — Photo
 * Library, Take Photo, Choose File. The app asks that question itself
 * (`SourceSheet`), with the library and Files going through the Rust dialog
 * and its grants, so this is only ever opened for the camera row.
 *
 * Hidden, and out of the tab order: it is opened by the control that renders
 * it, which is what carries the label.
 */
export default function FileInput({ testid, accept, capture, onFile, ref }: Props) {
  const changed = (event: ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0]
    // So the same file chosen twice fires a change both times.
    event.target.value = ''
    if (file) onFile(file)
  }

  return (
    <input
      ref={ref}
      type="file"
      accept={accept}
      capture={capture ? 'environment' : undefined}
      onChange={changed}
      data-testid={testid}
      className="hidden"
      tabIndex={-1}
      aria-hidden
    />
  )
}
