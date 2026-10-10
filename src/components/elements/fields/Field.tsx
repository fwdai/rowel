import { useState, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { cx } from '@/utils/cx'
import { copy } from '@/services/copy'
import type { TKey } from '@/i18n'
import { EyeGlyph, EyeOffGlyph } from '../../Main/icons'
import CopyButton from '../CopyButton'
import IconButton from '../IconButton'
import { verbatimInput } from '../inputProps'
import { HOVER_ONLY, MASK_DOTS, MASK_INPUT, VALUE_LINE } from '../tokens'
import { useField } from './context'
import { requiredError } from './formats'
import FieldRow from './Row'

export interface FieldProps {
  /** Draft key, and the input's `name` — the selector the e2e suite owns. */
  name: string
  /** Untranslated row label; omit for a full-bleed row. */
  label?: TKey
  /** Blocks the save while empty and marks the row after the first attempt. */
  required?: boolean
  /** Masked until the eye is pressed, in both modes. */
  secure?: boolean
  /** The row's headline secret: 2xl, letter-spaced for reading aloud. */
  big?: boolean
  type?: string
  maxLength?: number
  placeholder?: string
  prefix?: ReactNode
  /** Trailing controls shown in both modes. */
  actions?: ReactNode
  below?: ReactNode
  /** Stored → stored, on blur (a URL gains its missing scheme). */
  normalize?: (value: string) => string
  /**
   * Stored → shown, in both modes — an ISO date read in the user's pattern.
   * Paired with `normalize`, which takes what was typed back to storage, so the
   * editor shows and accepts the same form the read view does.
   */
  format?: (value: string) => string
  /**
   * Reading only: a muted gloss after the value, from the value itself — the
   * country an alpha-3 code names. Decoration, never storage: what is shown in
   * the editor and what the copy button hands over are both untouched.
   */
  suffix?: (value: string) => string | undefined
  /** A format complaint about a non-empty value, or ''. */
  check?: (value: string) => string
}

// One field, both modes. Reading, it is the detail row it has always been:
// the value, reveal toggle, copy button, hidden when empty. Editing, the same
// row with the value swapped for an underlined borderless input. Every
// type-aware field in this folder is a thin wrapper around it.
export default function Field({
  name,
  label,
  required,
  secure,
  big,
  type = 'text',
  maxLength,
  placeholder,
  prefix,
  actions,
  below,
  normalize,
  format,
  suffix,
  check
}: FieldProps) {
  const { t } = useTranslation()
  const { value, set, editing, attempted } = useField(name)
  // Typing into dots is guesswork, so the editor starts revealed; the eye still
  // takes it back.
  const [show, setShow] = useState(editing)
  // What the row puts on screen and on the clipboard; the draft keeps `value`.
  const shown = format ? format(value) : value

  // Reading, an empty field is not a field.
  if (!editing && value === '') return null

  // Reading is error-free by construction: `attempted` is only ever true in an
  // editing session, and a format complaint is an editor's business.
  const error =
    editing && value.trim()
      ? (check?.(value) ?? '')
      : requiredError(value, required, attempted)

  const masked = secure && !show
  // The editor hides its own text; a read value gets the fixed mask instead.
  const mask = masked ? MASK_INPUT : undefined
  // The headline treatment is for a secret being read, not for its mask.
  // A tier up on the phone (16px), where the desktop's 13 reads as a form; a
  // masked secret also takes the secret's tracking there, so the dots sit
  // apart the way the prototype draws them.
  const ink = cx(
    VALUE_LINE,
    big && !masked ? 'text-xl tracking-secret' : 'text-base max-md:text-lg',
    masked && 'max-md:tracking-secret'
  )
  // Nothing to gloss about a mask, and nothing to gloss while typing.
  const gloss = !editing && !masked ? suffix?.(value) : undefined

  return (
    <FieldRow
      label={label}
      prefix={prefix}
      below={below}
      error={error}
      actions={
        <>
          {actions}
          {secure && (
            <IconButton
              title={show ? t('Hide') : t('Reveal')}
              active={show}
              testid={`reveal-${name}`}
              onClick={() => setShow(!show)}
            >
              {show ? <EyeOffGlyph /> : <EyeGlyph />}
            </IconButton>
          )}
          {!editing && (
            // A row with a reveal has a control rail already; a plain one keeps
            // its copy button quiet until the row is hovered or focused. On the
            // phone there is no copy button at all: the value is the copy
            // control (below), the toast says it worked, and the screen's own
            // bottom button covers the main secret — as Passwords and
            // 1Password draw their rows. A rail there holds only what means
            // something else: the eye, the open-link.
            <span className={cx(!secure && HOVER_ONLY, 'max-md:hidden')}>
              <CopyButton value={shown} title={t('Copy')} />
            </span>
          )}
        </>
      }
    >
      {id =>
        editing ? (
        <input
          id={id}
          name={name}
          type={type}
          value={shown}
          placeholder={placeholder}
          maxLength={maxLength}
          {...verbatimInput}
          style={mask}
          onChange={event => set(event.target.value)}
          onBlur={normalize ? () => set(normalize(value)) : undefined}
          className={cx(
            ink,
            'w-full border-b bg-transparent text-text outline-none transition-colors placeholder:text-text2',
            error ? 'border-bad' : 'border-line2 focus:border-accent-line'
          )}
        />
        ) : (
          // The gloss rides alongside the value rather than inside it, so the
          // testid — and everything reading it — still holds the value alone.
          <span className="flex min-w-0 items-baseline gap-1.5">
            {/* The value IS the copy affordance: a finger cannot hover a row to
                find the button, and text selection is off app-wide, so there is
                nothing else a press on it could mean. A masked value hands over
                the real one, exactly as the button beside it already does. */}
            <button
              type="button"
              // Masked, the button's text is twelve dots and names nothing, so
              // it says what it is and what pressing it does instead.
              aria-label={label ? `${t(label)} · ${t('Copy')}` : t('Copy')}
              onClick={() => copy(shown)}
              className={cx(
                ink,
                'cursor-pointer text-left',
                masked ? 'text-text2 max-md:text-text' : 'text-text'
              )}
              data-testid={`entry-value-${name}`}
            >
              {masked ? MASK_DOTS : shown}
            </button>
            {gloss && (
              <span className="min-w-0 flex-none truncate text-base leading-6 text-text3 max-md:text-md">
                · {gloss}
              </span>
            )}
          </span>
        )
      }
    </FieldRow>
  )
}
