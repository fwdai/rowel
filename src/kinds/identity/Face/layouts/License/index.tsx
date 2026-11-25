import { useTranslation } from 'react-i18next'
import Eye from '@/components/elements/Face/Eye'
import { LicenseGlyph } from '@/components/Main/icons'
import { countryName } from '@/utils/countries'
import { DOC_TYPE_LABELS } from '../../../templates'
import DocCell from '../../DocCell'
import Medallion from '../../Medallion'
import Plate from '../../Plate'
import Portrait from '../../Portrait'
import Print from '../../Print'
import Status from '../../Status'
import { FIELDS, HEAD, NAME, SUBTITLE, TITLE } from '../../flow'
import type { LayoutProps } from '..'
import { INDEX } from './numbering'
import { STOCK } from './stock'

/**
 * A driving licence, as the card is laid out the world over: the issuing
 * state's plate in the corner beside the card's title, the photograph, and the
 * fields under the numbers the licence standard gives them — 1 the name, 3 the
 * date of birth, 4a to 4c the issue, the expiry and the authority, 5 the
 * licence number, 9 the categories it covers. On rose stock, the colour the card has had since it was
 * paper.
 */
export default function License({ docType, doc, flip }: LayoutProps) {
  const { t } = useTranslation()
  const country = doc.value('country')

  return (
    <div className="flex min-w-0 flex-1 flex-col px-5 pb-3.5 pt-4">
      <Print {...STOCK} />

      <Medallion line={STOCK.line} onFlip={flip}>
        <LicenseGlyph size={22} />
      </Medallion>

      <header className={HEAD}>
        <Plate doc={doc} />
        <div className="min-w-0 flex-1">
          <div data-testid="identity-doc-type" className={TITLE}>
            {t(DOC_TYPE_LABELS[docType])}
          </div>
          <div className={SUBTITLE}>{countryName(country) ?? country}</div>
        </div>
      </header>

      <div className="mt-3.5 flex items-start gap-4">
        <Portrait className="w-[24%]" />
        <div className={FIELDS}>
          <DocCell
            doc={doc}
            name="name"
            label="Holder"
            index={INDEX.name}
            ink={NAME}
            wrap
            className="w-full"
          />
          <DocCell doc={doc} name="birth_date" index={INDEX.birth_date} size={11} />
          <DocCell doc={doc} name="issue_date" index={INDEX.issue_date} size={11} />
          <DocCell doc={doc} name="expiry_date" index={INDEX.expiry_date} size={11} />
          <DocCell doc={doc} name="authority" index={INDEX.authority} size={16} />
          <DocCell doc={doc} name="categories" index={INDEX.categories} size={16} />
        </div>
      </div>

      <div className="mt-auto flex flex-none items-end gap-2.5 pt-3">
        <DocCell
          doc={doc}
          name="number"
          index={INDEX.number}
          ink="text-xl tracking-secret"
          wrap
          className="min-w-0 flex-1"
        />
        <Status iso={doc.value('expiry_date')} />
        <Eye shown={doc.shown} onToggle={doc.toggle} testid="reveal-number" className="mb-0.5" />
      </div>
    </div>
  )
}
