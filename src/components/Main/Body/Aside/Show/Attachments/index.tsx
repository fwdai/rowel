import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { Attachment, EntryMeta } from '@/api/types'
import ConfirmSheet from '@/components/elements/ConfirmSheet'
import { ACTION_ROW, EDGE_GUTTER, META, SECTION_LABEL_FLUSH } from '@/components/elements/tokens'
import { humanSize } from '@/utils/size'
import { useFileDrop } from '@/hooks/useFileDrop'
import AttachButton from './AttachButton'
import { attachTarget, isAttachDrop } from './drop'
import Preview from './Preview'
import Row from './Row'
import { useAttachments } from './useAttachments'

// How much of the vault's budget its files may take before the list says so.
const NEARLY_FULL = 0.8

const REMOVE_TITLE_ID = 'remove-attachment-title'

/**
 * The files attached to an entry, under its fields while reading.
 *
 * Disclosed only as far as there is something to show: an entry with no files
 * gets one quiet "Attach file" line and nothing else; once there is a file,
 * the section appears as a plain list of the files — each a line with its
 * thumbnail, name and size (`Row`), hairlines between, on the screen's own
 * ground with no card around them — and the same quiet line under it for the
 * next one. A press on a line opens the file (`Preview`), and the bytes come
 * to the webview only for the file being shown. The vault's budget for files
 * goes unmentioned until it is nearly spent, and then only under a list of
 * files. On a phone the attach action first asks where from — the camera, the
 * photo library or the file browser — since they are separate pickers there
 * (see `AttachButton`); a photo from the camera arrives as bytes, the other
 * two as a granted path.
 *
 * Which file is open is the section's, not the row's: the preview steps
 * through the list — the arrows on the desktop — so it has to know its
 * neighbours, and removing the open file moves it on to the next rather than
 * closing it, as Photos does. Removing asks first, from wherever it was asked
 * (a row's hover action, the preview's bar), through one `ConfirmSheet` here;
 * it stands over the preview, so Escape backs out of the question before it
 * closes the file.
 *
 * A file dropped on the window while this entry is open is attached to it,
 * unless the drop is the scanner's or the env flow's (see `drop.ts`). Both
 * shells mount this through `Show/Body`; the phone has nothing to drop, and
 * `useFileDrop` is a no-op there.
 */
export default function Attachments({ entry }: { entry: EntryMeta }) {
  const { t } = useTranslation()
  const { items, usage, error, busy, add, addFile, pick, save, remove } = useAttachments(
    entry.id
  )
  const [openId, setOpenId] = useState<string | null>(null)
  const [removing, setRemoving] = useState<Attachment | null>(null)

  useFileDrop(paths => {
    const [path] = paths
    if (path && isAttachDrop(path) && attachTarget()?.id === entry.id) void add(path)
  })

  // Derived, so a file another device removed meanwhile closes on its own.
  const openAt = items.findIndex(item => item.id === openId)
  const open = openAt >= 0 ? items[openAt] : null

  const step = (by: -1 | 1) => {
    const next = items[openAt + by]
    if (next) setOpenId(next.id)
  }

  const confirmRemove = () => {
    if (!removing) return
    // The open file goes: on to the one after it, back to the one before, or
    // closed when it was the last.
    if (removing.id === openId) setOpenId(items[openAt + 1]?.id ?? items[openAt - 1]?.id ?? null)
    setRemoving(null)
    remove(removing.id)
  }

  return (
    // One gutter for the heading, the lines, the verb and the notes: they all
    // stand on the pane outside any card, and align because they share it.
    <div data-testid="attachments" className={EDGE_GUTTER}>
      {items.length > 0 && (
        <div className="mt-4" data-testid="attachments-section">
          <span className={SECTION_LABEL_FLUSH}>{t('Attachments')}</span>
          <div>
            {items.map(item => (
              <Row
                key={item.id}
                attachment={item}
                onOpen={() => setOpenId(item.id)}
                onSave={() => save(item.id)}
                onRemove={() => setRemoving(item)}
              />
            ))}
          </div>
          {usage && usage.used > usage.limit * NEARLY_FULL && (
            <p className={`mt-2 ${META}`} data-testid="attachments-usage">
              {t('{{used}} of {{limit}} used by attachments in this vault', {
                used: humanSize(usage.used),
                limit: humanSize(usage.limit)
              })}
            </p>
          )}
        </div>
      )}
      <AttachButton
        busy={busy}
        onPick={pick}
        onFile={file => void addFile(file)}
        // Under a list the pill wants a row's worth of air, as the tags'
        // chips have under their label.
        className={`${items.length === 0 ? 'mt-4' : 'mt-3'} ${ACTION_ROW}`}
      />
      {error && (
        <p className="mt-2 text-base text-bad" role="alert">
          {error}
        </p>
      )}

      {open && (
        <Preview
          attachment={open}
          index={openAt}
          count={items.length}
          onStep={step}
          onSave={() => save(open.id)}
          onRemove={() => setRemoving(open)}
          onClose={() => setOpenId(null)}
        />
      )}

      {/* After the preview in the tree, so it is the dialog on top. */}
      {removing && (
        <ConfirmSheet
          id={REMOVE_TITLE_ID}
          testid="attachment-remove-sheet"
          actionTestid="attachment-remove-confirm"
          title={t('Remove “{{name}}”?', { name: removing.name })}
          message={t('The file is removed from this entry. This can’t be undone.')}
          action={t('Remove attachment')}
          onConfirm={confirmRemove}
          onClose={() => setRemoving(null)}
        />
      )}
    </div>
  )
}
