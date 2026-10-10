import { useTranslation } from 'react-i18next'
import { cx } from '@/utils/cx'
import CopyButton from '@/components/elements/CopyButton'
import IconButton from '@/components/elements/IconButton'
import {
  CopyValue,
  PHONE,
  PHONE_LABEL,
  PHONE_RAIL,
  PHONE_VALUE,
  RAIL,
  ROW_PAD,
  STACK,
  STACK_LABEL,
  STACK_RAIL
} from '@/components/elements/fields'
import {
  HOVER_ONLY,
  MASK_DOTS,
  ROW_COPY,
  ROW_HAIRLINE,
  VALUE
} from '@/components/elements/tokens'
import { EyeGlyph, EyeOffGlyph } from '@/components/Main/icons'
import type { EnvVar } from '../parse'
import { KEY_COL, STACK_VALUE } from './styles'

interface Props {
  v: EnvVar
  revealed: boolean
  onReveal: () => void
}

// One variable, read. The detail row's geometry with the label column swapped
// for the key — shown as the user wrote it, since it is their identifier, not
// a label of ours — and the value masked until asked for. Every value is
// masked: a `PORT` in plain would be convenient, but guessing which names are
// secrets shows `DB_HOST=user:pass@…` on a shared screen when it guesses wrong.
export default function Row({ v, revealed, onReveal }: Props) {
  const { t } = useTranslation()
  const masked = !revealed

  return (
    <div
      className={cx('group flex items-center gap-3', ROW_PAD, ROW_HAIRLINE, STACK, PHONE)}
    >
      <span
        data-testid={`env-key-${v.index}`}
        className={cx(KEY_COL, 'truncate text-text2', STACK_LABEL, PHONE_LABEL)}
      >
        {v.key}
      </span>
      <div className={cx('min-w-0 flex-1', PHONE_VALUE)}>
        <span className="flex min-w-0 items-baseline gap-1.5">
          {/* Masked, it hands over the real value, exactly as the button
              beside it does. */}
          <CopyValue
            value={v.value}
            label={`${v.key} · ${t('Copy')}`}
            testid={`env-value-${v.index}`}
            className={cx(
              VALUE,
              'font-mono',
              masked ? 'text-text2 max-md:text-text' : 'text-text',
              STACK_VALUE
            )}
          >
            {masked ? MASK_DOTS : v.value}
          </CopyValue>
          {/* Comments come from the same encrypted body as the value and may
              themselves contain secrets, so the row's eye governs both. */}
          {revealed && v.comment && (
            <span className="min-w-0 flex-none truncate text-base leading-6 text-text3">
              · {v.comment}
            </span>
          )}
        </span>
      </div>
      <div className={cx(RAIL, STACK_RAIL, PHONE_RAIL)}>
        <IconButton
          title={revealed ? t('Hide') : t('Reveal')}
          active={revealed}
          testid={`reveal-env-${v.index}`}
          onClick={onReveal}
        >
          {revealed ? <EyeOffGlyph /> : <EyeGlyph />}
        </IconButton>
        <span className={cx(HOVER_ONLY, ROW_COPY)}>
          <CopyButton value={v.value} title={t('Copy')} />
        </span>
      </div>
    </div>
  )
}
