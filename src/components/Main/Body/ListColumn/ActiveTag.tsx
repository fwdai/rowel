import { useTranslation } from 'react-i18next'
import { useUi, setView } from '@/store'
import TagChip from '@/components/elements/TagChip'

// The tag the list is narrowed by, under the search field: the same chip the
// entry wears, lit, with the × that lifts the filter.
export default function ActiveTag() {
  const { t } = useTranslation()
  const tag = useUi(state => (state.view === 'tags' ? state.filterTag : null))
  if (!tag) return null

  return (
    <div className="mt-2 flex">
      <TagChip
        tone="active"
        removes
        testid="active-tag"
        title={t('Clear tag filter')}
        onClick={() => setView('items')}
      >
        {tag}
      </TagChip>
    </div>
  )
}
