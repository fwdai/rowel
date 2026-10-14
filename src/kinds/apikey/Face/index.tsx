import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useCopied } from '@/hooks/useCopied'
import { useDates } from '@/hooks/useDates'
import { cx } from '@/utils/cx'
import Paper from '@/components/elements/Face'
import Cell from '@/components/elements/Face/Cell'
import FaceIconButton from '@/components/elements/Face/IconButton'
import { useField } from '@/components/elements/fields'
import Mask from '@/components/elements/Mask'
import { LABEL_TYPE } from '@/components/elements/tokens'
import { CheckGlyph, CopyGlyph, EyeGlyph, EyeOffGlyph } from '@/components/Main/icons'
import { ENVIRONMENT_LABELS, environmentOf } from '../environment'
import { hostPath, splitKey } from '../keyInfo'

/**
 * The key as the physical object — the read view's face of an API key.
 *
 * The token strip is the card: the issuer's prefix in plain sight, the rest
 * sealed until the eye is pressed, and a click on the strip copies the whole.
 * Under a rule, where the key is used — its environment, loud when it is
 * production, the API's address, and the day it lapses. Reading only: the
 * editor keeps its rows, where each of these has a place to be typed.
 */
export default function Face() {
  const { t } = useTranslation()
  const { copied, copy } = useCopied()
  const { formatDate } = useDates()
  const [shown, setShown] = useState(false)
  const key = useField('apiKey').value
  const environment = environmentOf(useField('environment').value.trim())
  const baseUrl = useField('baseUrl').value.trim()
  const expires = useField('expiry_date').value.trim()

  const { prefix, body } = splitKey(key)
  // Code points, not UTF-16 units: what a person would count.
  const size = [...key].length
  const foot = environment || baseUrl || expires

  return (
    // The vault's shared face (see elements/Face) in its slate tone, like the
    // SSH key's fingerprint plate: an object of the app, not of the world, so
    // it follows the theme where a document's paper does not.
    <Paper tone="slate" data-testid="apikey-face" className="flex flex-col p-5">
      <div className="flex flex-wrap items-center gap-2">
        <span className={`flex-1 ${LABEL_TYPE} text-(--face-ink2)`}>{t('Secret key')}</span>
        <span className="rounded-xs border border-(--face-rule) px-1.5 py-0.5 font-mono text-2xs tracking-label text-(--face-ink2)">
          {t('{{size}} chars', { size })}
        </span>
      </div>

      <div className="mt-5 flex items-start gap-2">
        <button
          type="button"
          onClick={() => copy(key)}
          title={t('Copy')}
          aria-label={`${t('Secret key')} · ${t('Copy')}`}
          className="-mx-1.5 min-w-0 flex-1 cursor-pointer rounded-sm px-1.5 py-1 text-left transition-colors hover:bg-(--face-hover)"
        >
          {/* Wraps rather than clips: a value you cannot see is worse than a
              strip two lines tall. Masked, the mask is a fixed length and says
              nothing about the key's. */}
          <span
            className="block break-all font-mono text-lg leading-[1.4] tracking-[0.06em]"
            data-testid="entry-value-apiKey"
          >
            {prefix && <span className="text-(--face-ink2)">{prefix}</span>}
            {shown ? body : <Mask />}
          </span>
        </button>
        <FaceIconButton
          title={shown ? t('Hide') : t('Reveal')}
          active={shown}
          testid="reveal-apiKey"
          onClick={() => setShown(!shown)}
        >
          {shown ? <EyeOffGlyph /> : <EyeGlyph />}
        </FaceIconButton>
        <FaceIconButton title={copied ? t('Copied') : t('Copy')} onClick={() => copy(key)}>
          {copied ? <CheckGlyph className="text-good" /> : <CopyGlyph />}
        </FaceIconButton>
      </div>

      {foot && (
        <div className="mt-5 flex flex-wrap gap-x-8 gap-y-3 border-t border-(--face-rule) pt-4">
          {environment && (
            <Cell
              name="environment"
              label="Environment"
              value={environment}
              display={t(ENVIRONMENT_LABELS[environment])}
              // Production is the one that costs something when mistaken for
              // the other, so it alone is coloured.
              ink={cx('text-base', environment === 'production' && 'text-bad')}
            />
          )}
          {baseUrl && (
            <Cell name="baseHost" label="Base URL" value={baseUrl} display={hostPath(baseUrl)} />
          )}
          {expires && (
            <Cell
              name="expiry_date"
              label="Expires"
              value={expires}
              display={formatDate(expires)}
              className="ml-auto text-right"
            />
          )}
        </div>
      )}
    </Paper>
  )
}
