import { useState, type KeyboardEvent } from 'react'
import { useTranslation } from 'react-i18next'
import type { ExtraField } from '@/api/types'
import { cx } from '@/utils/cx'
import { EyeGlyph, EyeOffGlyph, TrashGlyph } from '../../../Main/icons'
import CopyButton from '../../CopyButton'
import IconButton from '../../IconButton'
import { verbatimInput } from '../../inputProps'
import Mask from '../../Mask'
import {
  HOVER_ONLY,
  MASK_INPUT,
  ROW_COPY,
  ROW_HAIRLINE,
  ROW_LABEL,
  VALUE_LINE
} from '../../tokens'
import CopyValue from '../CopyValue'
import {
  PHONE,
  PHONE_LABEL,
  PHONE_RAIL,
  PHONE_SIGIL,
  PHONE_VALUE,
  RAIL,
  ROW_PAD,
  STACK,
  STACK_LABEL,
  STACK_RAIL,
  STACK_SIGIL
} from '../Row'

interface Props {
  field: ExtraField
  /** Position in the list: what the inputs are named and tested by. */
  index: number
  /** Editing only: writes this row back to the draft. */
  onChange?: (next: ExtraField) => void
  onRemove?: () => void
  /** Enter in the value box appends a row; offered only on the last one. */
  onAppend?: () => void
  /** A row just added takes the caret, so "Add field" is followed by typing. */
  autoFocus?: boolean
}

// The value column at the fixed rows' tiers in both modes (see Field): 13px
// on the desktop, 16px on the phone, and a mask spaced like a secret's there.
const ink = (masked: boolean) =>
  cx(VALUE_LINE, 'w-full text-base max-md:text-lg', masked && 'max-md:tracking-secret')

const BOX =
  'border-b bg-transparent outline-none transition-colors focus:border-accent-line'

/*
 * The editor's rail: the fixed rows' 60px two-slot rail on the desktop, and
 * on the phone the same two controls at a compact 32px with 16px glyphs —
 * two of the rail's 44px blocks side by side were a toolbar, and took 88px
 * off the value — pulled 8px into the row's padding so the glyphs sit near
 * the card's edge as the fixed rows' do. Each control still answers to a
 * 44px finger: the slop (`EDIT_CONTROL`'s `before`) reaches 6px past the box.
 * `STACK_RAIL` is not taken: it is what grows a rail's buttons to 44px.
 */
const EDIT_RAIL =
  'flex w-[60px] flex-none items-center justify-end gap-1 @max-[420px]:w-auto max-md:col-start-2 max-md:row-span-2 max-md:row-start-1 max-md:-mr-2 max-md:w-auto max-md:self-center max-md:[&_svg]:size-4'
const EDIT_CONTROL =
  "relative max-md:h-8 max-md:w-8 max-md:before:absolute max-md:before:-inset-1.5 max-md:before:content-['']"

// One label/value pair, in the detail row's geometry: the label takes the w-32
// label column the fixed rows use, the value the rest, the rail the controls.
// Reading, the value is the copy control like any other's, with a copy button
// beside it where there is a pointer to hover; editing, the label is typed
// too — it is the user's word for this field, not a translated one — in the
// fixed labels' face and ink, so the row still reads as a row. On the phone,
// where the label is the caption over the value, it has no underline at all,
// and "Field name" in its place is what says it is still to be filled.
//
// The editor's two controls are glyphs alone in the rail — conceal, remove —
// as every control in every rail is: no words, so nothing shifts when the
// toggle turns, and nothing under the value to make the row taller. They take
// the control ink the fixed rows' generate and eye take, no paler; the
// conceal eye lights in the accent once the value is concealed, and a
// concealed value is typed into dots like a password.
//
// A concealed pair reads like a secure Field: dots until the eye is pressed,
// its copy button always in sight. In a narrow container the row folds
// label-over-value as the fixed rows do, and on the phone it takes their grid
// (see FieldRow for both).
export default function CustomFieldRow({
  field,
  index,
  onChange,
  onRemove,
  onAppend,
  autoFocus
}: Props) {
  const { t } = useTranslation()
  const editing = !!onChange
  const [revealed, setRevealed] = useState(false)
  const masked = !!field.secret && (editing || !revealed)

  // Enter is inert in the editor (only ⌘⏎ saves), so the last row can spend it
  // on the next one — filling a list never needs the mouse.
  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key !== 'Enter' || !onAppend) return
    event.preventDefault()
    onAppend()
  }

  return (
    <div
      className={cx(
        'group flex items-center gap-3',
        ROW_PAD,
        STACK,
        PHONE,
        !editing && ROW_HAIRLINE
      )}
    >
      {onChange ? (
        <input
          name={`extra-label-${index}`}
          value={field.label}
          aria-label={t('Label')}
          placeholder={t('Field name')}
          maxLength={60}
          autoFocus={autoFocus}
          {...verbatimInput}
          onChange={event => onChange({ ...field, label: event.target.value })}
          className={cx(
            'w-32 flex-none border-line2 placeholder:text-text2 max-md:border-b-0',
            ROW_LABEL,
            BOX,
            STACK_LABEL,
            PHONE_LABEL
          )}
        />
      ) : (
        <span
          data-testid={`entry-extra-label-${index}`}
          className={cx('w-32 flex-none truncate', ROW_LABEL, STACK_LABEL, PHONE_LABEL)}
        >
          {field.label}
        </span>
      )}
      {/* The fixed rows' sigil slot (see FieldRow), held open so these values
          start where the rows above them do. */}
      <span className={cx('w-4 flex-none', STACK_SIGIL, PHONE_SIGIL)} />

      <div className={cx('min-w-0 flex-1', PHONE_VALUE)}>
        {onChange ? (
          <input
            name={`extra-value-${index}`}
            value={field.value}
            aria-label={t('Value')}
            placeholder={t('Value')}
            {...verbatimInput}
            style={masked ? MASK_INPUT : undefined}
            onChange={event => onChange({ ...field, value: event.target.value })}
            onKeyDown={onKeyDown}
            className={cx(ink(masked), 'text-text placeholder:text-text2', BOX, 'border-line2')}
          />
        ) : (
          <CopyValue
            value={field.value}
            label={`${field.label} · ${t('Copy')}`}
            testid={`entry-extra-value-${index}`}
            className={cx(ink(masked), masked ? 'text-text2 max-md:text-text' : 'text-text')}
          >
            {masked ? <Mask /> : field.value}
          </CopyValue>
        )}
      </div>

      {editing ? (
        <div className={EDIT_RAIL}>
          <IconButton
            title={field.secret ? t('Stop concealing') : t('Conceal value')}
            active={field.secret}
            testid={`conceal-extra-${index}`}
            onClick={() => onChange({ ...field, secret: !field.secret })}
            className={EDIT_CONTROL}
          >
            <EyeOffGlyph />
          </IconButton>
          <IconButton
            title={t('Remove field')}
            testid={`remove-extra-${index}`}
            onClick={onRemove}
            className={EDIT_CONTROL}
          >
            <TrashGlyph />
          </IconButton>
        </div>
      ) : (
        <div className={cx(RAIL, STACK_RAIL, PHONE_RAIL)}>
          {field.secret && (
            <IconButton
              title={revealed ? t('Hide') : t('Reveal')}
              active={revealed}
              testid={`reveal-extra-${index}`}
              onClick={() => setRevealed(!revealed)}
            >
              {revealed ? <EyeOffGlyph /> : <EyeGlyph />}
            </IconButton>
          )}
          {/* Quiet until the row is under the cursor or the keyboard, like
              the fixed rows' own copy button — bar a concealed one, which
              has a control rail already, as a secure Field does. */}
          <span className={cx(!field.secret && HOVER_ONLY, ROW_COPY)}>
            <CopyButton value={field.value} title={t('Copy')} />
          </span>
        </div>
      )}
    </div>
  )
}
