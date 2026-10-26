import { useTranslation } from 'react-i18next'
import type { EntryMeta } from '@/api/types'
import { favoriteEntries, restoreEntries } from '@/store'
import Button from '@/components/elements/Button'
import IconButton from '@/components/elements/IconButton'
import { ArchiveGlyph, ArchiveRestoreGlyph, StarGlyph, TrashGlyph } from '../../icons'
import type { Run } from './Bar'

interface Props {
  rows: EntryMeta[]
  archived: boolean
  /**
   * The destructive verb has had its first press: it turns red, and the bar
   * asks the question in the count's place (see Bar).
   */
  armed: boolean
  busy: boolean
  run: Run
  destroy: () => void
}

// The verbs a batch has, the same ones an entry's own header offers: in a
// live view the star and Archive, in the Archive the last delete and Restore.
export default function Actions({ rows, archived, armed, busy, run, destroy }: Props) {
  const { t } = useTranslation()

  if (archived)
    return (
      <>
        <Button
          variant={armed ? 'danger' : 'pale'}
          size="md"
          disabled={busy}
          onClick={destroy}
          testid={armed ? 'batch-purge-confirm' : 'batch-purge'}
        >
          <TrashGlyph />
          {t('Delete')}
        </Button>
        <Button size="md" disabled={busy} onClick={() => run(restoreEntries)} testid="batch-restore">
          <ArchiveRestoreGlyph />
          {t('Restore')}
        </Button>
      </>
    )

  // One star for the batch, lit once every row wears one: pressing it then
  // takes them all off, and otherwise stars the rest.
  const starred = rows.every(row => row.favorite)

  return (
    <>
      {/* Armed, the bar is a question with one answer; the star waits. */}
      {!armed && (
        <IconButton
          title={starred ? t('Remove from Favorites') : t('Add to Favorites')}
          active={starred}
          disabled={busy}
          onClick={() => run(ids => favoriteEntries(ids, !starred))}
          testid="batch-favorite"
        >
          <StarGlyph filled={starred} />
        </IconButton>
      )}
      <Button
        variant={armed ? 'danger' : 'pale'}
        size="md"
        disabled={busy}
        onClick={destroy}
        testid={armed ? 'batch-archive-confirm' : 'batch-archive'}
      >
        <ArchiveGlyph />
        {t('Archive')}
      </Button>
    </>
  )
}
