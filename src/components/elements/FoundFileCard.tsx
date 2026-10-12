import { useTranslation } from 'react-i18next'
import { CARD, META, META_TYPE } from '@/components/elements/tokens'
import { CloudGlyph, DiskGlyph } from '@/components/Main/icons'

interface Props {
  /** Where the file is: the cloud it syncs through, or the disk it sits on. */
  where: 'cloud' | 'disk'
  name: string
  /** The line under the name — how old it is, how big. */
  meta?: string
  /** The green "encrypted" chip: true of anything the app wrote. */
  encrypted?: boolean
  testid?: string
}

// The file the flow found, named and dated so the next step is about *this*
// one: the Drive pack the probe came back with, or the backup just picked.
export default function FoundFileCard({ where, name, meta, encrypted, testid }: Props) {
  const { t } = useTranslation()
  const cloud = where === 'cloud'

  return (
    <div
      data-testid={testid}
      className={`${CARD} flex items-center gap-3 px-3.5 py-3 text-left`}
    >
      <span
        className={`grid h-9 w-9 flex-none place-items-center rounded-sm bg-tile ${
          cloud ? 'text-accent' : 'text-text2'
        }`}
      >
        {cloud ? <CloudGlyph size={18} /> : <DiskGlyph size={18} />}
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-base font-medium text-text">{name}</span>
        {meta && <span className={`mt-0.5 block truncate ${META}`}>{meta}</span>}
      </span>
      {encrypted && (
        <span className={`flex flex-none items-center gap-1.5 ${META_TYPE} text-good`}>
          <span aria-hidden className="h-1.5 w-1.5 rounded-full bg-current" />
          {t('encrypted')}
        </span>
      )}
    </div>
  )
}
