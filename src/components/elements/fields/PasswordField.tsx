import { useTranslation } from 'react-i18next'
import type { TFunction } from 'i18next'
import { openGenerator } from '@/store'
import type { TKey } from '@/i18n'
import { useDates } from '@/hooks/useDates'
import { relativeDuration, toTime, type Dates } from '@/utils/time'
import { RefreshGlyph } from '../../Main/icons'
import IconButton from '../IconButton'
import StrengthBar from '../StrengthBar'
import Field from './Field'
import PasswordHistory from './PasswordHistory'
import FieldRow from './Row'
import { useField, useFields } from './context'
import { META } from '../tokens'

// How long this password has been in place. `now` reads as "just now" rather
// than "changed now ago", and past a week — where the duration runs out — the
// date goes in a sentence of its own rather than "changed 05/06/2026 ago".
const rotationStamp = (
  t: TFunction,
  { shortDate }: Dates,
  iso?: string | string[]
): string => {
  if (typeof iso !== 'string') return ''
  const at = toTime(iso)
  if (at === null) return ''
  const ago = relativeDuration(iso)
  if (ago === 'now') return t('Changed just now')
  if (ago) return t('Changed {{ago}} ago', { ago })
  return t('Changed on {{date}}', { date: shortDate(at) })
}

export default function PasswordField({
  name = 'password',
  label = 'Password',
  required
}: {
  name?: string
  label?: TKey
  required?: boolean
}) {
  const { t } = useTranslation()
  const { entry } = useFields()
  const { value, set, editing } = useField(name)
  const stamp = rotationStamp(t, useDates(), entry.password_updated_at)
  // Reading only: the editor is about the password being set, not the old ones.
  // The stamp is a sentence about the password, not a label for one — and,
  // once there are previous ones, the way in to them.
  const history =
    !editing && entry.id && entry.passwordHistory?.length ? (
      <PasswordHistory id={entry.id} history={entry.passwordHistory} stamp={stamp} />
    ) : null

  // A cleared password still has the ones before it, so the row outlives its
  // value: nothing to show, reveal or copy, only the way in to the history.
  if (!editing && !value && history)
    return (
      <FieldRow label={label} below={history}>
        {() => null}
      </FieldRow>
    )

  return (
    <Field
      name={name}
      label={label}
      required={required}
      secure
      big
      maxLength={100}
      placeholder="••••••••"
      actions={
        editing ? (
          <IconButton
            title={t('Generate')}
            testid="generate-password-link"
            onClick={() => openGenerator(set)}
          >
            <RefreshGlyph />
          </IconButton>
        ) : undefined
      }
      below={
        (value || stamp) && (
          <>
            <StrengthBar password={value} />
            {history ?? (stamp && <span className={META}>{stamp}</span>)}
          </>
        )
      }
    />
  )
}
