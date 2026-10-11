import { CheckGlyph } from '@/components/Main/icons'
import { cx } from '@/utils/cx'
import { META, ROW_HAIRLINE } from '../tokens'

interface Props {
  tag: string
  /** How many entries carry it across the vault; 0 for one made just now. */
  count: number
  on: boolean
  onToggle: (on: boolean) => void
}

/**
 * One tag the entry can be filed under: the name leading, how many entries
 * already carry it, and a check when this one does. The row of the system's
 * own pickers (Files' tags, Reminders'), 52px for the thumb, the card's
 * hairline between rows. A checkbox to assistive technology, which is what
 * it is: on or off, and nothing happens elsewhere when it is pressed.
 */
export default function Row({ tag, count, on, onToggle }: Props) {
  return (
    <div className={ROW_HAIRLINE}>
      <button
        type="button"
        role="checkbox"
        aria-checked={on}
        data-testid={`tag-pick-${tag}`}
        onClick={() => onToggle(!on)}
        className="flex h-13 w-full cursor-pointer items-center gap-3 px-4 text-left text-lg text-text transition-colors hover:bg-hover active:bg-hover"
      >
        <span className="min-w-0 flex-1 truncate">{tag}</span>
        {count > 0 && (
          <span className={cx('flex-none', META)} data-testid={`tag-pick-${tag}-count`}>
            {count}
          </span>
        )}
        <span
          aria-hidden
          className={cx(
            'grid w-5 flex-none place-items-center text-accent transition-opacity',
            !on && 'opacity-0'
          )}
        >
          <CheckGlyph size={18} stroke={2.25} />
        </span>
      </button>
    </div>
  )
}
