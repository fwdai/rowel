import { useRef } from 'react'
import { useTranslation } from 'react-i18next'
import { CameraGlyph, FolderGlyph, ImageGlyph } from '../Main/icons'
import ActionRow from './ActionRow'
import DialogHeader from './DialogHeader'
import FileInput from './FileInput'
import Frame from './Frame'
import { CARD } from './tokens'

interface Props {
  /** The prefix of every test id in the sheet: `<testid>-sheet`, `-camera`… */
  testid: string
  title: string
  /** The line under the title: what will happen to what is chosen. */
  description: string
  /** Backing out: the scrim, the grabber, the close control, or Escape. */
  onClose: () => void
  /** A photo the camera just took, as the file the webview holds. */
  onCapture: (file: File) => void
  /** The photo library was chosen; the caller opens it from Rust. */
  onLibrary: () => void
  /** Files was chosen; the caller opens it from Rust. */
  onBrowse: () => void
}

/**
 * Where a photo or a file comes from, asked the way a phone asks it: a sheet
 * from the bottom edge, one row per source under the name the system gives
 * it — Take Photo, Photo Library, Choose File — each opening the system's own
 * picker for the answer. It is the Add picker's sheet, with the same header
 * and the same rows as its "other ways in", so the two read as one surface;
 * and it is never stacked on that picker: a row there that needs this closes
 * the picker and opens this in its place, as Receive does.
 *
 * The camera is the webview's own capture input (`FileInput`), which iOS opens
 * on the camera outright; what it takes is a `File` on no disk, and travels
 * as bytes. The library and Files are the dialog run from Rust, which grants
 * the path it hands back (`pickFileToRead`). A system picker backed out of
 * leaves the question open; the camera's photo answers it.
 *
 * `fit="content"`: three rows are little enough to answer from the bottom
 * edge. Only the phone shell opens this; the desktop goes straight to a file
 * dialog whose own sidebar reaches the library.
 */
export default function SourceSheet({
  testid,
  title,
  description,
  onClose,
  onCapture,
  onLibrary,
  onBrowse
}: Props) {
  const { t } = useTranslation()
  const camera = useRef<HTMLInputElement>(null)
  const titleId = `${testid}-title`

  return (
    <Frame
      onClose={onClose}
      labelledBy={titleId}
      testid={`${testid}-sheet`}
      fit="content"
      hideClose
    >
      <FileInput
        ref={camera}
        testid={`${testid}-capture`}
        accept="image/*"
        capture
        onFile={onCapture}
      />
      <DialogHeader
        id={titleId}
        title={title}
        description={description}
        onClose={onClose}
        className="px-5"
      />
      <div className="px-5 pt-1 pb-2">
        <div className={CARD}>
          <ActionRow
            testid={`${testid}-camera`}
            glyph={<CameraGlyph size={16} />}
            label={t('Take Photo')}
            onClick={() => camera.current?.click()}
          />
          <ActionRow
            testid={`${testid}-library`}
            glyph={<ImageGlyph size={16} />}
            label={t('Photo Library')}
            onClick={onLibrary}
          />
          <ActionRow
            testid={`${testid}-browse`}
            glyph={<FolderGlyph size={16} />}
            label={t('Choose File…')}
            onClick={onBrowse}
          />
        </div>
      </div>
    </Frame>
  )
}
