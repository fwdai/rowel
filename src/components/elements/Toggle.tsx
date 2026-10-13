import { cx } from '@/utils/cx'

interface Props {
  checked: boolean
  onChange: (next: boolean) => void
  disabled?: boolean
  name?: string
  testid?: string
  'aria-label'?: string
}

// THE on/off control for settings rows: a 40x22 pill whose track carries the
// accent when on. The knob is white on the grey off-track and the accent's own
// foreground on the on-track — the accent is ink, white on the dark theme, so a
// white knob there would vanish into it. A native `button role="switch"`
// handles Space/Enter and the global `:focus-visible` ring for free.
//
// Where a finger is one of the pointers it grows to the system switch's own
// 51x31 (`any-pointer-coarse`, as every touch tier in the app keys off), so a
// phone's settings row is not asking a thumb to hit a 22px pill.
export default function Toggle({
  checked,
  onChange,
  disabled,
  name,
  testid,
  'aria-label': ariaLabel
}: Props) {
  return (
    <button
      type="button"
      role="switch"
      name={name}
      aria-checked={checked}
      aria-label={ariaLabel}
      disabled={disabled}
      data-testid={testid}
      onClick={() => onChange(!checked)}
      className={cx(
        'relative h-[22px] w-10 flex-none cursor-pointer rounded-full transition-colors any-pointer-coarse:h-[31px] any-pointer-coarse:w-[51px]',
        checked ? 'bg-accent' : 'bg-line2',
        disabled && 'cursor-default opacity-50'
      )}
    >
      <span
        className={cx(
          'absolute top-[2px] left-[2px] h-[18px] w-[18px] rounded-full transition-[transform,background-color] any-pointer-coarse:h-[27px] any-pointer-coarse:w-[27px]',
          checked
            ? 'translate-x-[18px] bg-accent-fg any-pointer-coarse:translate-x-5'
            : 'bg-white'
        )}
      />
    </button>
  )
}
