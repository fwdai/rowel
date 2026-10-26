import { useVault, selectBatch } from '@/store'
import { cx } from '@/utils/cx'
import ListColumn from './ListColumn'
import SortMenu from './List/SortMenu'
import BatchBar from './Batch'
import DetailPane from './DetailPane'

// The two content panes to the right of the rail: list column + detail pane.
// Creating or editing an entry happens inside the detail pane, so the list
// stays visible for context but goes quiet and inert — nothing over there can
// compete with, or navigate away from, an unsaved draft.
export default function Body() {
  const writing = useVault(state => state.editing || !!state.creating)
  const batch = useVault(selectBatch)

  return (
    <>
      {/* `inert` rather than `pointer-events-none`: the latter only stops the
          mouse, leaving the column's own arrows and ⏎ free to re-select a row
          and end the edit session out from under a dirty draft. */}
      <div
        data-testid="list-column"
        inert={writing || undefined}
        className={cx(
          'relative flex min-h-0 w-[348px] flex-none border-r border-line',
          writing && 'opacity-60'
        )}
      >
        {/* While the batch bar floats over the column's foot, the list keeps
            its last rows scrollable out from under it. */}
        <ListColumn actions={<SortMenu />} scroller={batch ? 'pb-14' : undefined} />
        <BatchBar />
      </div>
      <DetailPane />
    </>
  )
}
