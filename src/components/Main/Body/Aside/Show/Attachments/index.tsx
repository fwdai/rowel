import { useTranslation } from 'react-i18next'
import type { EntryMeta } from '@/api/types'
import { ACTION_ROW, EDGE_GUTTER, META, SECTION_LABEL_FLUSH } from '@/components/elements/tokens'
import { humanSize } from '@/utils/size'
import { useFileDrop } from '@/hooks/useFileDrop'
import AttachButton from './AttachButton'
import { attachTarget, isAttachDrop } from './drop'
import Row from './Row'
import { useAttachments } from './useAttachments'

// How much of the vault's budget its files may take before the list says so.
const NEARLY_FULL = 0.8

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

  useFileDrop(paths => {
    const [path] = paths
    if (path && isAttachDrop(path) && attachTarget()?.id === entry.id) void add(path)
  })

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
                onSave={() => save(item.id)}
                onRemove={() => remove(item.id)}
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
        className={`${items.length === 0 ? 'mt-4' : 'mt-1'} ${ACTION_ROW}`}
      />
      {error && (
        <p className="mt-2 text-base text-bad" role="alert">
          {error}
        </p>
      )}
    </div>
  )
}
