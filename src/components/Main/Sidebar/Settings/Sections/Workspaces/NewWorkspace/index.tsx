import { useTranslation } from 'react-i18next'
import { ChevronRightGlyph, PlusGlyph } from '@/components/Main/icons'
import { useSubpage } from '../../../sectionNav'

// The list's last row: the way to one more workspace, on a sub-page of its own.
// The card it closes leaves its overflow open (for the rows' menus), so the row
// rounds its own bottom corners to match.
export default function NewWorkspace() {
  const { t } = useTranslation()
  const { open } = useSubpage()

  return (
    <button
      type="button"
      data-testid="workspace-new-row"
      onClick={() => open({ key: 'new-workspace' })}
      className="flex w-full cursor-pointer items-center gap-3.5 rounded-b-lg px-4 py-3.5 text-left transition-colors hover:bg-hover max-md:rounded-b-xl"
    >
      <span className="grid h-9 w-9 flex-none place-items-center rounded-lg border-[1.5px] border-dashed border-accent-line text-accent">
        <PlusGlyph />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block text-base font-medium text-accent">{t('New lockbox')}</span>
        <span className="mt-0.5 block text-sm text-text2">
          {t('A separate encrypted lockbox for work, family or a client')}
        </span>
      </span>
      <ChevronRightGlyph className="flex-none text-text2" />
    </button>
  )
}
