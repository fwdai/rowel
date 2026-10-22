import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useCopied } from '@/hooks/useCopied'
import { useDates } from '@/hooks/useDates'
import { cx } from '@/utils/cx'
import Copied from '@/components/elements/Copied'
import Paper, { PRESS } from '@/components/elements/Face'
import Cell from '@/components/elements/Face/Cell'
import CopyHint from '@/components/elements/Face/CopyHint'
import Eye from '@/components/elements/Face/Eye'
import FaceHeader from '@/components/elements/Face/Header'
import { useField } from '@/components/elements/fields'
import Mask from '@/components/elements/Mask'
import { ENVIRONMENT_LABELS, environmentOf } from '../environment'
import { hostPath, splitKey } from '../keyInfo'

/**
 * The key as the physical object — the read view's face of an API key.
 *
 * The token strip is the card: the issuer's prefix in plain sight, the rest
 * sealed until the eye is pressed, and a press on the strip copies the whole,
 * as a press on any value on a face does. Under a rule, where the key is used —
 * its environment, loud when it is production, the API's address, and the day
 * it lapses. Reading only: the editor keeps its rows, where each of these has
 * a place to be typed.
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
      <FaceHeader label="Secret key" chip={t('{{size}} chars', { size })} />

      {/* The strip and, beside it, the face's one control: the eye. The copy is
          the strip itself, hinted at under the pointer and confirmed in place,
          and the header's primary action above the face. */}
      <div className="mt-5 flex items-start gap-2">
        <button
          type="button"
          onClick={() => copy(key)}
          title={t('Copy')}
          aria-label={`${t('Secret key')} · ${t('Copy')}`}
          className={cx(PRESS, '-mx-1.5 min-w-0 flex-1 px-1.5 py-1')}
        >
          {/* Wraps rather than clips: a value you cannot see is worse than a
              strip two lines tall. Masked, the mask is a fixed length and says
              nothing about the key's. */}
          <span
            className="relative block break-all font-mono text-lg leading-[1.4] tracking-[0.06em]"
            data-testid="entry-value-apiKey"
          >
            <Copied copied={copied} ink="text-(--face-ink)">
              {prefix && <span className="text-(--face-ink2)">{prefix}</span>}
              {shown ? body : <Mask />}
            </Copied>
          </span>
          <CopyHint />
        </button>
        <Eye shown={shown} onToggle={() => setShown(!shown)} testid="reveal-apiKey" />
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
              // the other, so it alone is coloured — in the face's own verdict
              // ink, which the theme lifts for a dark slab.
              ink={cx(
                'text-base leading-tight',
                environment === 'production' && 'text-(--face-bad)'
              )}
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
