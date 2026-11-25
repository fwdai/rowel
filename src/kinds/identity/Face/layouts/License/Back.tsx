import { useTranslation } from 'react-i18next'
import { CAPTION } from '@/components/elements/Face/Cell'
import { specOf, type IdentityKey } from '../../../templates'
import type { BackProps } from '..'
import Reverse, { Watermark } from '../Reverse'
import { INDEX } from './numbering'

// The bars of the licence's barcode. Decoration, like the portrait's
// silhouette: the vault holds no barcode, and what the code would carry is
// printed on the front in full.
const BARS =
  'repeating-linear-gradient(90deg, #1B1D21 0 2px, transparent 2px 4px, #1B1D21 4px 5px, transparent 5px 9px, #1B1D21 9px 12px, transparent 12px 14px)'

const NUMBERED = Object.entries(INDEX) as [IdentityKey, string][]

// The categories as the front holds them, one code at a time. The row is free
// text, so whatever a person put between the codes — commas, slashes, spaces —
// is taken as the gap between them.
const codes = (categories: string): string[] => categories.split(/[\s,;/|]+/).filter(Boolean)

/**
 * The back of a driving licence, where the card says what its holder may
 * drive: each category the front lists in a box of its own, as the table on a
 * real licence sets them, over the legend to the numbers the front's fields
 * are set under, with the card's mark large in the stock and the barcode along
 * the foot.
 */
export default function LicenseBack({ docType, doc, stock }: BackProps) {
  const { t } = useTranslation()
  const held = codes(doc.value('categories'))

  return (
    <Reverse
      docType={docType}
      doc={doc}
      band={
        <div className="border-t border-(--face-rule) bg-white/70 px-5 py-3">
          <div aria-hidden className="h-9 opacity-70" style={{ backgroundImage: BARS }} />
        </div>
      }
    >
      <div className="flex min-w-0 flex-1 flex-col gap-3">
        {held.length > 0 && (
          <ul data-testid="identity-categories" className="flex flex-wrap gap-1.5">
            {held.map((code, at) => (
              <li
                key={at}
                className="grid h-7 min-w-9 place-items-center rounded-[5px] border bg-white/60 px-2 text-base font-semibold uppercase tracking-[0.04em]"
                style={{ borderColor: stock.line }}
              >
                {code}
              </li>
            ))}
          </ul>
        )}
        <ol className={`grid grid-cols-2 gap-x-5 gap-y-1.5 ${CAPTION} text-(--face-ink2)`}>
          {NUMBERED.map(([key, index]) => (
            <li key={key} className="flex min-w-0 gap-2">
              <span className="flex-none font-semibold normal-case text-(--face-ink)">{index}</span>
              {/* The name is captioned "Holder" on the front, so it is here too. */}
              <span className="truncate">{t(key === 'name' ? 'Holder' : specOf(key).label)}</span>
            </li>
          ))}
        </ol>
      </div>
      <Watermark stock={stock} doc={doc} />
    </Reverse>
  )
}
