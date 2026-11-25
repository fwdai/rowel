import { useTranslation } from 'react-i18next'
import Eye from '@/components/elements/Face/Eye'
import { FingerprintGlyph } from '@/components/Main/icons'
import { countryName } from '@/utils/countries'
import { DOC_TYPE_LABELS } from '../../../templates'
import DocCell from '../../DocCell'
import Medallion from '../../Medallion'
import Portrait from '../../Portrait'
import Print from '../../Print'
import Term from '../../Term'
import { FIELDS, HEAD, NAME, SUBTITLE, TITLE } from '../../flow'
import type { LayoutProps } from '..'
import { STOCK } from './stock'

/**
 * A residence permit: the granting state's code set large in the corner beside
 * the card's title, the print of a finger as its mark, the holder beside the
 * photograph — a permit's holder is another state's national, so the
 * nationality leads — with the card's number under them, and the term the
 * permit runs for drawn along the foot. On green stock.
 */
export default function Permit({ docType, doc, flip }: LayoutProps) {
  const { t } = useTranslation()
  const country = doc.value('country')

  return (
    <div className="flex min-w-0 flex-1 flex-col px-5 pb-3.5 pt-4">
      <Print {...STOCK} />

      <Medallion line={STOCK.line} onFlip={flip}>
        <FingerprintGlyph size={24} />
      </Medallion>

      <header className={HEAD}>
        <DocCell
          doc={doc}
          name="country"
          bare
          ink="text-xl font-semibold uppercase tracking-[0.06em]"
          size={4}
          className="flex-none"
        />
        <div className="min-w-0 flex-1">
          <div data-testid="identity-doc-type" className={TITLE}>
            {t(DOC_TYPE_LABELS[docType])}
          </div>
          <div className={SUBTITLE}>{countryName(country) ?? country}</div>
        </div>
      </header>

      <div className="mt-3.5 flex items-start gap-4">
        <Portrait />
        <div className={FIELDS}>
          <DocCell doc={doc} name="name" label="Holder" ink={NAME} wrap className="w-full" />
          <DocCell doc={doc} name="nationality" size={5} />
          <DocCell doc={doc} name="birth_date" size={11} />
          <DocCell doc={doc} name="authority" size={12} />
          <div className="flex w-full items-end gap-2.5">
            <DocCell
              doc={doc}
              name="number"
              ink="text-xl tracking-secret"
              wrap
              className="min-w-0 flex-1"
            />
            <Eye shown={doc.shown} onToggle={doc.toggle} testid="reveal-number" className="mb-0.5" />
          </div>
        </div>
      </div>

      <div className="mt-auto flex-none pt-3">
        <Term doc={doc} />
      </div>
    </div>
  )
}
