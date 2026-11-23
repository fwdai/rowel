import type { ReactNode } from 'react'
import { cx } from '@/utils/cx'
import { ROW_HAIRLINE } from '@/components/elements/tokens'
import { ChevronRightGlyph } from '@/components/Main/icons'

interface Props {
  /** At row size (16px), in a dashed tile where a lockbox's monogram would be. */
  icon: ReactNode
  label: string
  description: string
  onClick: () => void
  testid: string
}

// The row under a list that leads to one more of what it lists, on a sub-page
// of its own. The card it closes leaves its overflow open (for the rows'
// menus), so the row rounds its own bottom corners to match.
export default function AddRow({ icon, label, description, onClick, testid }: Props) {
  return (
    <button
      type="button"
      data-testid={testid}
      onClick={onClick}
      className={cx(
        'flex w-full cursor-pointer items-center gap-3.5 px-4 py-3.5 text-left transition-colors hover:bg-hover last:rounded-b-lg max-md:last:rounded-b-xl',
        ROW_HAIRLINE
      )}
    >
      <span className="grid h-9 w-9 flex-none place-items-center rounded-lg border-[1.5px] border-dashed border-accent-line text-accent">
        {icon}
      </span>
      <span className="min-w-0 flex-1">
        <span className="block text-base font-medium text-accent">{label}</span>
        <span className="mt-0.5 block text-sm text-text2">{description}</span>
      </span>
      <ChevronRightGlyph className="flex-none text-text2" />
    </button>
  )
}
