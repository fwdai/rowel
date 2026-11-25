import { useTranslation } from 'react-i18next'
import { DOC_TYPE_LABELS, type DocType } from '../templates'

/** What kind of document this is, where its front says so. */
export default function TypeLabel({ docType }: { docType: DocType }) {
  const { t } = useTranslation()
  return <span data-testid="identity-doc-type">{t(DOC_TYPE_LABELS[docType])}</span>
}
