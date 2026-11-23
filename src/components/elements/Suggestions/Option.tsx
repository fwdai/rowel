import { cx } from '@/utils/cx'
import { META_TYPE } from '../tokens'
import { matchAt, type Suggestion } from './match'

interface Props {
  id: string
  option: Suggestion
  /** What is in the field, so the option can show where it answers to it. */
  typed: string
  lit: boolean
  onLight: () => void
  onPick: () => void
}

// The value with the stretch that matched in the full ink and a step heavier,
// and the rest a step back, so the eye lands on why this one is here. The lit
// one is read whole.
function Value({ value, typed, lit }: { value: string; typed: string; lit: boolean }) {
  const length = typed.trim().length
  const at = length ? matchAt(value, typed) : -1
  if (at < 0) return <span className="min-w-0 flex-1 truncate">{value}</span>

  return (
    <span className={cx('min-w-0 flex-1 truncate transition-colors', !lit && 'text-text2')}>
      {value.slice(0, at)}
      <span className="font-medium text-text">{value.slice(at, at + length)}</span>
      {value.slice(at + length)}
    </span>
  )
}

// One value on offer: the value, and at the end how many items already hold
// it. Lit in the accent's wash, so the list follows the colour the user chose.
export default function Option({ id, option, typed, lit, onLight, onPick }: Props) {
  return (
    <button
      type="button"
      id={id}
      role="option"
      aria-selected={lit}
      tabIndex={-1}
      data-testid={`suggestion-${option.value}`}
      onMouseEnter={onLight}
      onClick={onPick}
      className={cx(
        'flex h-8 w-full flex-none cursor-pointer items-center gap-5 rounded-[6px] px-2.5 text-left text-base text-text max-md:text-md any-pointer-coarse:h-10',
        lit && 'bg-accent-soft'
      )}
    >
      <Value value={option.value} typed={typed} lit={lit} />
      {option.count !== undefined && (
        <span className={cx('flex-none', META_TYPE, lit ? 'text-text2' : 'text-text3')}>
          {option.count}
        </span>
      )}
    </button>
  )
}
