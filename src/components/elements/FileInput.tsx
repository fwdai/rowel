import type { ChangeEvent, Ref } from 'react'

interface Props {
  testid: string
  /** The input's `accept`: `image/*` for a photo, left out for any file. */
  accept?: string
  /** What the user chose. Not called when the system's sheet is dismissed. */
  onFile: (file: File) => void
  /** The input itself, for the caller to `click()` — nothing else opens it. */
  ref: Ref<HTMLInputElement>
}

/**
 * The phone's way of asking where a file comes from: the webview's own file
 * input, which iOS answers with the system's sheet — Photo Library, Take Photo,
 * Choose File — and then the system's own picker for the answer. Nothing of
 * the sheet is drawn here, which is the point: it is the one every other app
 * on the phone shows, under the names the system gives its sources, over
 * whatever asked, with the system's Cancel to back out. The camera's
 * permission prompt is the system's too (the usage string is in the iOS
 * `Info.plist`); the library and Files need none.
 *
 * What it hands back is a `File` on no path the backend could be pointed at,
 * so it travels as bytes (`scan_image_bytes`). No `capture`: that would skip
 * the sheet and open the camera outright. And only where the camera belongs
 * in the question — attaching a file asks with `ActionSheet` instead, since
 * this sheet cannot be told to leave the camera out.
 *
 * Hidden, and out of the tab order: it is opened by the control that renders
 * it, which is what carries the label.
 */
export default function FileInput({ testid, accept, onFile, ref }: Props) {
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
      onChange={changed}
      data-testid={testid}
      className="hidden"
      tabIndex={-1}
      aria-hidden
    />
  )
}
