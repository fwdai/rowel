import { useState, type KeyboardEvent } from 'react'
import { useTranslation } from 'react-i18next'
import type { ExtraField } from '@/api/types'
import { cx } from '@/utils/cx'
import { EyeGlyph, EyeOffGlyph, TrashGlyph } from '../../../Main/icons'
import CopyButton from '../../CopyButton'
import IconButton from '../../IconButton'
import { verbatimInput } from '../../inputProps'
import {
  HOVER_ONLY,
  LABEL_TYPE,
  MASK_DOTS,
  MASK_INPUT,
  ROW_COPY,
  ROW_HAIRLINE,
  ROW_LABEL,
  VALUE
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
}

// The same value column in both modes, so switching does not move the row.
const INK = `${VALUE} w-full`
const BOX =
  'border-b border-line2 bg-transparent outline-none transition-colors placeholder:text-text2 focus:border-accent-line'

// One label/value pair, in the detail row's geometry: the label takes the w-32
// label column the fixed rows use, the value the rest. Reading, the value is
// the copy control like any other's, with a copy button beside it where there
// is a pointer to hover; editing, the label is typed too — it is the user's
// word for this field, not a translated one — and the row can be dropped.
//
// A concealed pair reads like a secure Field: dots until the eye is pressed,
// its copy button always in sight. Editing, the eye is what conceals it, and
// a concealed value is typed into dots like a password. In a narrow container
// the row folds label-over-value as the fixed rows do, and on the phone it
// takes their grid (see FieldRow for both).
export default function CustomFieldRow({
  field,
  index,
  onChange,
  onRemove,
  onAppend
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
          placeholder={t('Label')}
          maxLength={60}
          {...verbatimInput}
          onChange={event => onChange({ ...field, label: event.target.value })}
          className={cx('w-32 flex-none text-text', LABEL_TYPE, BOX, STACK_LABEL, PHONE_LABEL)}
        />
      ) : (
        <span
          data-testid={`entry-extra-label-${index}`}
          className={cx('w-32 flex-none truncate', ROW_LABEL, STACK_LABEL, PHONE_LABEL)}
        >
          {field.label}
        </span>
      )}
      {/* The fixed rows' sigil slot and actions slot (see FieldRow), held open
          so these values start and end where the rows above them do. */}
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
            className={cx(INK, 'text-text', BOX)}
          />
        ) : (
          <CopyValue
            value={field.value}
            label={`${field.label} · ${t('Copy')}`}
            testid={`entry-extra-value-${index}`}
            className={cx(INK, masked ? 'text-text2 max-md:text-text' : 'text-text')}
          >
            {masked ? MASK_DOTS : field.value}
          </CopyValue>
        )}
      </div>

      <div className={cx(RAIL, STACK_RAIL, PHONE_RAIL)}>
        {editing ? (
          <>
            <IconButton
              title={field.secret ? t('Stop concealing') : t('Conceal value')}
              active={field.secret}
              muted={!field.secret}
              testid={`conceal-extra-${index}`}
              onClick={() => onChange({ ...field, secret: !field.secret })}
            >
              <EyeOffGlyph />
            </IconButton>
            <IconButton
              title={t('Remove field')}
              testid={`remove-extra-${index}`}
              onClick={onRemove}
            >
              <TrashGlyph />
            </IconButton>
          </>
        ) : (
          <>
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
          </>
        )}
      </div>
    </div>
  )
}
