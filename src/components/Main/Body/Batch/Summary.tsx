import { useTranslation } from 'react-i18next'
import type { EntryMeta } from '@/api/types'
import { useUi } from '@/store'
import { kindOf } from '@/kinds'
import { KIND_TINT } from '@/kinds/tint'
import { chord, isPC } from '@/lib/platform'
import { cx } from '@/utils/cx'
import EmptyState from '@/components/elements/EmptyState'
import { PaneSurface } from '../Empty'
import { usePickedRows } from './usePickedRows'

// From the top of the pile down: the first picked row square on top, the two
// under it a step smaller and fanned out to either side.
const FAN = ['', '-translate-x-5 -rotate-[14deg] scale-90', 'translate-x-5 rotate-[14deg] scale-90']

// The batch as a pile of what is in it, the way Mail and Finder draw a
// multiple selection: up to three of the picked rows' kind tiles.
function Pile({ rows }: { rows: EntryMeta[] }) {
  const top = rows.slice(0, FAN.length)
  return (
    <div className="relative h-16 w-16">
      {top.map((row, depth) => {
        const { Glyph, tint } = kindOf(row.type)
        return (
          <div
            key={row.id}
            style={{ zIndex: top.length - depth }}
            // The tints are washes; the solid ground under each keeps the
            // tiles below from showing through.
            className={cx(
              'absolute inset-0 overflow-hidden rounded-xl border border-line bg-detail shadow-card',
              FAN[depth]
            )}
          >
            <div className={cx('grid h-full w-full place-items-center', KIND_TINT[tint])}>
              <Glyph size={26} />
            </div>
          </div>
        )
      })}
    </div>
  )
}

// What the detail pane shows while a batch is picked: no entry is open, so it
// says what is picked and how to change the pick. The verbs are on the bar.
export default function Summary() {
  const { t } = useTranslation()
  const rows = usePickedRows()
  const archived = useUi(state => state.view === 'archive')

  return (
    <PaneSurface>
      <EmptyState
        testid="batch-summary"
        mark={<Pile rows={rows} />}
        title={t('{{count}} items selected', { count: rows.length })}
        hints={[
          { keys: isPC ? 'Ctrl+click' : '⌘-click', label: t('add or remove') },
          { keys: isPC ? 'Shift+click' : '⇧-click', label: t('select a range') },
          { keys: chord('A'), label: t('select all') },
          { keys: isPC ? 'Del' : '⌘⌫', label: archived ? t('delete forever') : t('archive') },
          { keys: 'esc', label: t('deselect') }
        ]}
      />
    </PaneSurface>
  )
}
