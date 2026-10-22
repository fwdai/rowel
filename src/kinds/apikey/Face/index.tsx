import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useDates } from '@/hooks/useDates'
import { cx } from '@/utils/cx'
import Paper from '@/components/elements/Face'
import Cell from '@/components/elements/Face/Cell'
import Eye from '@/components/elements/Face/Eye'
import FaceHeader from '@/components/elements/Face/Header'
import { useField, useFields } from '@/components/elements/fields'
import Mask from '@/components/elements/Mask'
import { ENVIRONMENT_LABELS, environmentOf } from '../environment'
import Environment from '../Fields/Environment'
import { hostPath, splitKey } from '../keyInfo'

/**
 * The key as the physical object — the face of an API key, in both modes.
 *
 * The token strip is the card: the issuer's prefix in plain sight, the rest
 * sealed until the eye is pressed, and a press on the strip copies the whole,
 * as a press on any value on a face does. Under a rule, where the key is used —
 * its environment, loud when it is production, the API's address, and the day
 * it lapses. Editing, the strip is the box the key is pasted into and the foot
 * is where its environment and expiry are set, in the same places; the
 * address is typed in full in the row under the card and printed here.
 */
export default function Face() {
  const { t } = useTranslation()
  const { format, formatDate, toIsoDate } = useDates()
  const { set } = useFields()
  const editing = !!set
  // Pasted rather than typed, so the editor starts revealed like every other
  // secret's; the same eye takes it back in either mode.
  const [shown, setShown] = useState(editing)
  const key = useField('apiKey')
  const environment = environmentOf(useField('environment').value.trim())
  const baseUrl = useField('baseUrl').value.trim()
  const expires = useField('expiry_date')

  const { prefix, body } = splitKey(key.value)
  // Code points, not UTF-16 units: what a person would count.
  const size = [...key.value].length
  const foot = editing || environment || baseUrl || expires.value.trim()

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
        <Cell
          name="apiKey"
          label="Secret key"
          caption=""
          value={key.value}
          // Wraps rather than clips: a value you cannot see is worse than a
          // strip two lines tall. Masked, the mask is a fixed length and says
          // nothing about the key's.
          display={
            <>
              {prefix && <span className="text-(--face-ink2)">{prefix}</span>}
              {shown ? body : <Mask />}
            </>
          }
          onChange={editing ? key.set : undefined}
          placeholder="sk_live_…"
          required
          masked={!shown}
          wrap
          ink="font-mono text-lg leading-[1.4] tracking-[0.06em]"
          className="min-w-0 flex-1"
        />
        <Eye shown={shown} onToggle={() => setShown(!shown)} testid="reveal-apiKey" />
      </div>

      {foot && (
        <div className="mt-5 flex flex-wrap gap-x-8 gap-y-3 border-t border-(--face-rule) pt-4">
          {editing ? (
            <Environment />
          ) : (
            environment && (
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
            )
          )}
          {baseUrl && (
            <Cell name="baseHost" label="Base URL" value={baseUrl} display={hostPath(baseUrl)} />
          )}
          {(editing || expires.value.trim()) && (
            <Cell
              name="expiry_date"
              label="Expires"
              value={expires.value.trim()}
              display={formatDate(expires.value)}
              // Read and typed in the user's own date pattern, stored as ISO.
              box={formatDate(expires.value)}
              onChange={editing ? expires.set : undefined}
              normalize={toIsoDate}
              placeholder={format}
              maxLength={10}
              size={11}
              className="ml-auto text-right"
            />
          )}
        </div>
      )}
    </Paper>
  )
}
