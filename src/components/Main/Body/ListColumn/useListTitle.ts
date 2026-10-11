import { useTranslation } from 'react-i18next'
import { useUi, type View } from '@/store'
import { kindOf } from '@/kinds'
import type { TKey } from '@/i18n'

// The column is titled after the view, except in All Items, where an open kind
// renames it to what it is now showing ("Logins"), and in Tags, where the tag
// does ("work" — the name as the entry wears it, no `#`: a tag is not a
// hashtag). The other views keep their own name: "Logins" would lose the fact
// that you are looking at the Archive.
const TITLES: Record<View, TKey> = {
  items: 'All Items',
  favorites: 'Favorites',
  health: 'Vault Health',
  archive: 'Archive',
  tags: 'Tags'
}

// A hook rather than a constant in the column, because the compact shell draws
// the same name in its own large title: one computation, two headers.
export const useListTitle = (): string => {
  const { t } = useTranslation()
  const view = useUi(state => state.view)
  const type = useUi(state => state.filterType)
  const tag = useUi(state => state.filterTag)

  if (view === 'items' && type) return t(kindOf(type).pluralLabel)
  if (view === 'tags' && tag) return tag
  return t(TITLES[view])
}
