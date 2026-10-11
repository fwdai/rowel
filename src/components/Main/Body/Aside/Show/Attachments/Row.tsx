import { useState } from 'react'
import type { Attachment } from '@/api/types'
import { META, ROW_HAIRLINE } from '@/components/elements/tokens'
import { fileCaption, previewKind } from './kind'
import Preview from './Preview'
import Thumb from './Thumb'
import { useThumbnail } from './useAttachmentUrl'

interface Props {
  attachment: Attachment
  onSave: () => void
  onRemove: () => void
}

/**
 * One attached file, as a line of a plain list: a 40px thumbnail — the
 * picture itself for an image, the kind's page for anything else — beside
 * its name over its size and type, a hairline to the next, and nothing
 * around it. The list sits on the screen's own ground, as a list of files
 * does in Mail or in Notes' attachment view; a card would make a panel of
 * what is really a short inventory, and a grid of tiles would make a gallery
 * of what is really a vault. The whole line is a press that opens the file
 * (`Preview`); it carries no other control, so a finger and a pointer take
 * the same one path to save or remove.
 *
 * An image row draws the backend's small thumbnail of its file, never the
 * file: the file is read only when the row is opened, so a list of pictures
 * costs the webview kilobytes whatever the files weigh.
 */
export default function Row({ attachment, onSave, onRemove }: Props) {
  const [open, setOpen] = useState(false)
  const thumbnail = useThumbnail(attachment, previewKind(attachment) === 'image')

  const remove = () => {
    setOpen(false)
    onRemove()
  }

  return (
    <div data-testid="attachment-row" className={ROW_HAIRLINE}>
      <button
        type="button"
        data-testid="attachment-open-button"
        onClick={() => setOpen(true)}
        // No inset of its own: the list stands inside the section's
        // `EDGE_GUTTER`, with its heading and the verb under it.
        className="flex w-full min-w-0 cursor-pointer items-center gap-3 py-2.5 text-left transition-colors hover:bg-hover active:bg-hover"
      >
        <Thumb attachment={attachment} url={thumbnail} size="row" />
        <span className="min-w-0 flex-1">
          <span className="block truncate text-base text-text max-md:text-md">
            {attachment.name}
          </span>
          <span className={`mt-0.5 block ${META}`}>{fileCaption(attachment)}</span>
        </span>
      </button>
      {open && (
        <Preview
          attachment={attachment}
          onSave={onSave}
          onRemove={remove}
          onClose={() => setOpen(false)}
        />
      )}
    </div>
  )
}
