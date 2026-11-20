import { useTranslation } from 'react-i18next'
import { DropdownItem } from '../Dropdown'
import Kbd from '../Kbd'
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

// The value with the stretch that matched in the full ink and a heavier
// weight, and the rest a step back, so the eye lands on why this one is here.
function Value({ value, typed }: { value: string; typed: string }) {
  const length = typed.trim().length
  const at = length ? matchAt(value, typed) : -1
  if (at < 0) return <span className="truncate text-[13.5px] text-text">{value}</span>

  return (
    <span className="truncate text-[13.5px] text-text2">
      {value.slice(0, at)}
      <span className="font-semibold text-text">{value.slice(at, at + length)}</span>
      {value.slice(at + length)}
    </span>
  )
}

// One value on offer, drawn the way the lockbox picker draws a lockbox: a tile,
// the value over what is known about it (how many items already hold it), and
// at the end of the lit one the key that takes it.
export default function Option({ id, option, typed, lit, onLight, onPick }: Props) {
  const { t } = useTranslation()

  return (
    <DropdownItem
      id={id}
      option
      active={lit}
      testid={`suggestion-${option.value}`}
      onMouseEnter={onLight}
      onClick={onPick}
      className="gap-3 py-[7px]"
    >
      {option.glyph && (
        <span
          aria-hidden
          className="grid h-7 w-7 flex-none place-items-center rounded-sm bg-tile text-text2"
        >
          {option.glyph}
        </span>
      )}
      <span className="flex min-w-0 flex-1 flex-col gap-0.5 leading-tight">
        <Value value={option.value} typed={typed} />
        {option.count !== undefined && (
          <span className="truncate text-[12px] text-text2">
            {t('{{count}} items', { count: option.count })}
          </span>
        )}
      </span>
      {/* Held open on every row so lighting one moves nothing; a finger has no
          Enter to be told about. */}
      <span
        aria-hidden
        className={`flex-none any-pointer-coarse:hidden ${lit ? '' : 'invisible'}`}
      >
        <Kbd>⏎</Kbd>
      </span>
    </DropdownItem>
  )
}
