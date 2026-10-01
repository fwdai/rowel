import { useTranslation } from 'react-i18next'
import type { EntryMeta } from '@/api/types'
import AddAction from '@/components/elements/AddAction'
import Panel from '@/components/elements/Panel'
import { LABEL } from '@/components/elements/tokens'
import { useFileDrop } from '@/hooks/useFileDrop'
import { attachTarget, isAttachDrop } from './drop'
import AttachmentRow from './Row'
import { useAttachments } from './useAttachments'

/**
 * The files attached to an entry, under its fields while reading.
 *
 * Disclosed only as far as there is something to show: an entry with no files
 * gets one quiet "Attach file" and nothing else; the section with its label
 * and rows appears once there is a file to list. Add, list, save, remove —
 * nothing here opens a file or shows one, because the bytes never come to the
 * webview at all.
 *
 * A file dropped on the window while this entry is open is attached to it,
 * unless the drop is the scanner's or the env flow's (see `drop.ts`). Both
 * shells mount this through `Show/Body`; the phone has nothing to drop, and
 * `useFileDrop` is a no-op there.
 */
export default function Attachments({ entry }: { entry: EntryMeta }) {
  const { t } = useTranslation()
  const { items, error, busy, add, save, remove } = useAttachments(entry.id)

  useFileDrop(paths => {
    const [path] = paths
    if (path && isAttachDrop(path) && attachTarget()?.id === entry.id) void add(path)
  })

  return (
    <div data-testid="attachments">
      {items.length > 0 && (
        <div className="mt-4" data-testid="attachments-section">
          <span className={`mb-1.5 block ${LABEL}`}>{t('Attachments')}</span>
          <Panel>
            {items.map(item => (
              <AttachmentRow
                key={item.id}
                attachment={item}
                onSave={() => save(item.id)}
                onRemove={() => remove(item.id)}
              />
            ))}
          </Panel>
        </div>
      )}
      <AddAction
        label={busy ? t('Attaching…') : t('Attach file')}
        testid="attach-file-button"
        onClick={() => void add()}
        className={items.length === 0 ? 'mt-4 px-1' : 'mt-2 px-1'}
      />
      {error && (
        <p className="mt-2 px-1 text-base text-bad" role="alert">
          {error}
        </p>
      )}
    </div>
  )
}
