import { cx } from '@/utils/cx'
import { useRadioNav } from '@/hooks/useRadioNav'

interface Props<T extends string> {
  options: { value: T; label: string }[]
  value: T
  onChange: (value: T) => void
  // Names the radiogroup for screen readers — usually the row's own label,
  // which is otherwise only visually associated with the control.
  name?: string
  testidPrefix?: string
  className?: string
}

// THE two-to-five-way switch (generator mode, timeout, theme): a tile trough
// with the active segment raised on the lens — the same "you are here" key the
// rail and the phone tab bar use, so a chosen segment is a place rather than a
// wash of accent. Labels arrive already translated. On the phone it spans its
// row with the segments sharing the width, as the system's does, and where a
// finger is a pointer the segments are 44px tall.
export default function Segmented<T extends string>({
  options,
  value,
  onChange,
  name,
  testidPrefix,
  className
}: Props<T>) {
  // Destructured rather than kept as one `nav` object: reading `nav.ref` in the
  // render body is a ref access as far as `react-hooks/refs` is concerned.
  const { ref, onKeyDown } = useRadioNav(
    options.map(option => option.value),
    value,
    onChange
  )
  // With no segment active — an unset row, a value the options do not name —
  // the first one holds the tab stop, or the group could not be reached at all.
  const selected = options.some(option => option.value === value)

  return (
    <div
      ref={ref}
      role="radiogroup"
      aria-label={name}
      onKeyDown={onKeyDown}
      className={cx(
        'flex gap-0.5 rounded-lg bg-tile p-[3px] max-md:w-full max-md:[&>button]:flex-1',
        className
      )}
    >
      {options.map((option, index) => {
        const active = option.value === value
        return (
          <button
            key={option.value}
            type="button"
            role="radio"
            aria-checked={active}
            tabIndex={active || (!selected && index === 0) ? 0 : -1}
            data-testid={testidPrefix && `${testidPrefix}-${option.value}`}
            onClick={() => onChange(option.value)}
            className={cx(
              'h-[26px] cursor-pointer whitespace-nowrap rounded-sm px-2.5 text-base tabular-nums transition-[color,background-color,box-shadow] any-pointer-coarse:h-11',
              active
                ? 'bg-lens font-medium text-text shadow-lens'
                : 'text-text2 hover:text-text'
            )}
          >
            {option.label}
          </button>
        )
      })}
    </div>
  )
}
