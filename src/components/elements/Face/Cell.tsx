import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import type { TKey } from '@/i18n'
import { cx } from '@/utils/cx'
import { useCopied } from '@/hooks/useCopied'
import Copied from '../Copied'
import { useFields } from '../fields/context'
import { verbatimInput } from '../inputProps'
import { MASK_INPUT, META_TYPE } from '../tokens'
import { PRESS } from '.'
import CopyHint from './CopyHint'

// A face is a miniature, so its captions sit one step under the detail rows'
// 11px label tier and are tracked a little tighter. A caption is also what makes
// a field as wide as it is — "PERSONAL NO." is wider than any number under it —
// so this size is what decides how many fields fit across a document.
export const CAPTION = 'text-2xs uppercase tracking-[0.1em]'

interface Props {
  /** The draft key: the input's `name`, and `entry-value-<name>` when read. */
  name: string
  /** Untranslated: the caption over the value, and what names the control. */
  label?: TKey
  /**
   * A caption that is not the label — the hash a fingerprint is cut with,
   * already spelled. Given, it replaces the label as the caption ('' for none)
   * while the label still names the control.
   */
  caption?: string
  /** What is stored, and what a click copies. */
  value: string
  /** What is shown, when that differs: a date in the user's pattern, a mask. */
  display?: ReactNode
  /**
   * The value line's type, leading included. The name and the number are the
   * big ones; a digest is the mono tier.
   */
  ink?: string
  /** Wrap rather than truncate: a document number is never worth clipping. */
  wrap?: boolean
  /** A marker beside the read value — the expiry's "Expired" pill. */
  flag?: ReactNode
  /** The read value's selector, when it is not `entry-value-<name>`. */
  testid?: string
  className?: string

  /**
   * Editing. Given a writer, the cell is the input in the same place — the
   * face IS the form. Left off, a cell on a face being edited is printed, not
   * pressed: a derived value has nothing to type.
   */
  onChange?: (typed: string) => void
  /** What the box holds while editing, when not the value: a grouped number. */
  box?: string
  /** Already spelled: a catalogue string, a date pattern, or a dot mask. */
  placeholder?: string
  maxLength?: number
  /**
   * The cell's width in characters, in both modes — the box's width while
   * editing, and the least the printed value takes while reading, so the
   * cells flow the same way on the face whichever mode it is in. Without it
   * the cell is as wide as its line.
   */
  size?: number
  /** Blocks the save while empty and marks the box after the first attempt. */
  required?: boolean
  /** Overrides the box's own emptiness test (a half-typed MM/YY). */
  invalid?: boolean
  /** Typed → stored, on blur: a date back to ISO. */
  normalize?: (typed: string) => string
  /** The box hides its text, as the face's eye says. */
  masked?: boolean
}

// What both faces of the value line wear: the input draws its underline, the
// read value a transparent one, so the two are the same height to the pixel.
const LINE = 'relative block min-w-0 border-b'

// One printed value on a face: a micro-label, the value, and a copy on click —
// the same gesture every face teaches, confirmed in place by the shared mark
// in the face's own ink (see `Copied`), hinted at by the glyph in the corner.
// Editing, the same slot is the input, caption and all, in the face's inks.
export default function Cell({
  name,
  label,
  caption,
  value,
  display = value,
  ink = 'text-base leading-tight',
  wrap,
  flag,
  testid,
  className,
  onChange,
  box,
  placeholder,
  maxLength,
  size,
  required,
  invalid,
  normalize,
  masked
}: Props) {
  const { t } = useTranslation()
  const { set, attempted } = useFields()
  const { copied, copy } = useCopied()
  const named = label ? t(label) : caption
  const head = caption ?? named
  const cap = head && <span className={`block truncate ${CAPTION} text-(--face-ink2)`}>{head}</span>
  const pad = '-mx-1.5 -my-0.5 min-w-0 px-1.5 py-0.5'
  // One measure for both faces of the line, in the line's own type.
  const width = size ? `${size}ch` : undefined

  if (set && onChange) {
    const bad = invalid ?? (required && attempted && !value.trim())
    const held = box ?? value
    return (
      <label className={cx('block', pad, className)}>
        {cap}
        <input
          name={name}
          value={held}
          placeholder={placeholder}
          maxLength={maxLength}
          aria-label={head ? undefined : named}
          {...verbatimInput}
          style={{ ...(masked ? MASK_INPUT : {}), width }}
          onChange={event => onChange(event.target.value)}
          onBlur={normalize ? () => onChange(normalize(held)) : undefined}
          className={cx(
            LINE,
            'truncate bg-transparent outline-none transition-colors placeholder:text-(--face-ink2)',
            !size && 'w-full',
            bad ? 'border-(--face-bad)' : 'border-(--face-rule) focus:border-(--face-ink2)',
            ink
          )}
        />
        {bad && <span className={`mt-1 block ${META_TYPE} text-(--face-bad)`}>{t('Required')}</span>}
      </label>
    )
  }

  const line = (
    <span
      className={cx(LINE, 'border-transparent', wrap ? 'break-all' : 'truncate', ink)}
      style={{ minWidth: width }}
      data-testid={testid ?? `entry-value-${name}`}
    >
      <Copied copied={copied} ink="text-(--face-ink)">
        {display}
      </Copied>
    </span>
  )
  const body = flag ? (
    <span className="flex min-w-0 items-center gap-1.5">
      {line}
      {flag}
    </span>
  ) : (
    line
  )

  // Nothing to take: a value not yet there, dimmed, or a derived one printed
  // on a face being edited.
  if (!value || set)
    return (
      <div className={cx(pad, !value && 'text-(--face-ink2)', className)}>
        {cap}
        {body}
      </div>
    )

  return (
    <button
      type="button"
      onClick={() => copy(value)}
      title={t('Copy')}
      aria-label={named ? `${named} · ${t('Copy')}` : t('Copy')}
      className={cx(PRESS, pad, className)}
    >
      {cap}
      {body}
      <CopyHint copied={copied} />
    </button>
  )
}
