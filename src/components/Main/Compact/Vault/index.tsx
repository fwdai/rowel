import ListColumn from '../../Body/ListColumn'
import Title from '../../Body/ListColumn/Title'
import SortMenu from '../../Body/List/SortMenu'
import Audit from '../../Body/Aside/Audit'
import { DetailEmpty } from '../../Body/Empty'
import { useVariant, isWholeView } from '../../Body/Empty/variant'
import Tags from '../../Sidebar/Tags'
import { FIELD_ACTION, HEADER_ACTION, ROOT_HEADER, TAB_BAR_CLEARANCE } from '../chrome'

// The 44px search field's measure. Passed as classes rather than asked for by
// a flag: the box is the same field, dressed for a finger — its surface,
// states and motion are the field's own (`ListColumn/Search`). 8px under the
// title row, not 16: the row is already 56px tall around a 24px title, and
// the list under the field is what the screen is for. The right padding is
// the 4px inset of the tag tile set into the field's end.
const SEARCH = 'mt-2 h-11 gap-2.5 rounded-lg pl-3.5 pr-1 [&_input]:text-md'

/**
 * The list root — the screen the tab bar comes home to.
 *
 * The shared list column, with the phone's chrome around it: a 24px title
 * where the desktop has a 20px one, the rail's sort tile in the title row as
 * on the desktop (there is no rail; the rail's Add went to the tab bar's
 * centre), the tag filter set into the end of the search field — it narrows
 * the list the way a query does, so it lives with the query — and room at the
 * bottom of the scroller for the floating bar the rows slide under.
 */
export default function Vault() {
  // The list is the only pane here, so it also carries whatever the wide shell
  // puts in its detail pane — otherwise an empty vault is a blank screen, and
  // the audit view is its groups with no score.
  const variant = useVariant()

  // The same call `Body/Aside` makes for the wide detail pane: no variant means
  // the audit has a score to show, and the audit is the only view that can be
  // scored. A filter-shaped empty (`kind`, `search`, `select`) belongs to the
  // list column, which draws it among the rows itself.
  const footer = variant ? (
    isWholeView(variant) && <DetailEmpty variant={variant} />
  ) : (
    // The rows run edge to edge; the score panel wants the screen's gutters.
    <div className="px-4">
      <Audit />
    </div>
  )

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-screen pt-[env(safe-area-inset-top)]">
      <ListColumn
        heading={<Title className="text-2xl" />}
        header={ROOT_HEADER}
        actions={<SortMenu className={HEADER_ACTION} />}
        search={SEARCH}
        // The tile sits in the field rather than on a rail, so its menu drops
        // below the field's edge instead of out to a side: `top-full` is the
        // tile's bottom, 4px inside the field's.
        filter={<Tags className={FIELD_ACTION} menu="right-0 top-full mt-3" />}
        scroller={TAB_BAR_CLEARANCE}
        footer={footer}
      />
    </div>
  )
}
