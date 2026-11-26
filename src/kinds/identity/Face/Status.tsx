import { useTranslation } from 'react-i18next'
import { LABEL_TYPE } from '@/components/elements/tokens'
import { relativeFuture } from '@/utils/time'
import { validityInk } from './validity'

/*
 * Whether the document is still good, said once and in the margin — beside the
 * number, or at the end of a permit's term. Every date cell used to carry its
 * own countdown, which stated the same fact twice on any document that printed
 * both an issue and an expiry date. A document with no expiry says nothing.
 */
export default function Status({ iso }: { iso: string }) {
  const { t } = useTranslation()
  const ink = validityInk(iso)
  if (!ink) return null

  return (
    <span
      style={{ color: ink, borderColor: ink }}
      className={`flex h-5 flex-none items-center gap-1.5 rounded-xs border px-1.5 ${LABEL_TYPE}`}
    >
      <span aria-hidden className="h-1 w-1 rounded-full bg-current" />
      {relativeFuture(iso) || t('Expired')}
    </span>
  )
}
