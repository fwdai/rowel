import { useTranslation } from 'react-i18next'
import IconButton from '@/components/elements/IconButton'
import Segmented from '@/components/elements/Segmented'
import { CAPTION } from '@/components/elements/Face/Cell'
import { useField } from '@/components/elements/fields'
import { CloseGlyph } from '@/components/Main/icons'
import { ENVIRONMENT_LABELS, ENVIRONMENTS, environmentOf } from '../environment'

// Which environment the key is honoured in, on the face's foot while editing —
// a switch rather than a box, since there are two answers, under the caption
// the read cell wears. The environment is optional, so a set one can be
// cleared again: a slip of the switch must not leave PRODUCTION over a key
// that is nothing of the kind.
export default function Environment() {
  const { t } = useTranslation()
  const { value, set, editing } = useField('environment')
  const environment = environmentOf(value)

  if (!editing) return null

  return (
    <div className="min-w-0">
      <span className={`block ${CAPTION} text-(--face-ink2)`}>{t('Environment')}</span>
      <div className="mt-1 flex items-center gap-1">
        <Segmented
          options={ENVIRONMENTS.map(environment => ({
            value: environment,
            label: t(ENVIRONMENT_LABELS[environment])
          }))}
          // Unset, or a value the two do not name, lights no segment.
          value={environment ?? ''}
          onChange={set}
          name={t('Environment')}
          testidPrefix="apikey-environment"
          className="w-fit"
        />
        {environment && (
          <IconButton title={t('Clear')} testid="clear-environment" onClick={() => set('')}>
            <CloseGlyph />
          </IconButton>
        )}
      </div>
    </div>
  )
}
