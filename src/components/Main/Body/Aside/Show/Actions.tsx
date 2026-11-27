import type { Entry, EntryMeta } from '@/api/types'
import { editEntry, openSend } from '@/store'
import { useTranslation } from 'react-i18next'
import Button from '@/components/elements/Button'
import { CheckGlyph } from '../../../icons'
import Archived from './Archived'
import MoreMenu from './MoreMenu'
import { usePrimaryAction } from './usePrimaryAction'

interface Props {
  entry: EntryMeta
  // The decrypted entry, or null while `revealEntry` is still in flight.
  revealed: Entry | null
  /** The pane is holding this entry up for another: ⏎ must not copy from it. */
  settling?: boolean
  onDelete: () => void
}

// The detail header's action cluster: an overflow menu (Edit, Share, Archive)
// and the per-type primary copy action — or, for a tombstone, Restore and the
// last delete. The phone shell spreads the same two across its nav row and
// bottom button.
export default function Actions({ entry, revealed, settling, onDelete }: Props) {
  const { t } = useTranslation()
  const { label, secret, copied, copy } = usePrimaryAction(entry, revealed, !settling)

  // A tombstone is read-only, so it swaps the whole cluster rather than greying
  // parts of it out: nothing here applies to a row that is already archived.
  if (entry.deletedAt) return <Archived entry={entry} />

  return (
    <div className="flex flex-none items-center gap-1.5">
      <MoreMenu
        onDelete={onDelete}
        onEdit={() => editEntry()}
        onShare={() => openSend(entry.id)}
      />

      <Button size="md" kbd="⏎" disabled={!secret} onClick={copy} testid="primary-action-button">
        {copied ? (
          <>
            <CheckGlyph />
            {t('Copied')}
          </>
        ) : (
          t(label)
        )}
      </Button>
    </div>
  )
}
