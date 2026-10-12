import { useTranslation } from 'react-i18next'
import { isMobile } from '@/lib/platform'
import { pickImportFile } from '@/api/pickers'
import { useFileDrop } from '@/hooks/useFileDrop'
import { DownloadGlyph } from '../../../../icons'
import { META_TYPE } from '@/components/elements/tokens'
import HeroDropZone from './HeroDropZone'

interface Props {
  onDrop: (path: string) => void
  // `hero`: the section's lead drop target, with a picker button of its own
  // on every platform (see HeroDropZone). Left out, the compact strip below.
  variant?: 'hero'
}

export default function DropZone({ onDrop, variant }: Props) {
  return variant === 'hero' ? <HeroDropZone onDrop={onDrop} /> : <Strip onDrop={onDrop} />
}

// The "I don't see my provider" route into the same import: any export file,
// with the backend sniffing its format. Nothing drags a file onto a phone, so
// there the drop target becomes the button it was the alternative to — the
// picker is `pick_import_file`, the very one the tiles above already use.
function Strip({ onDrop }: Omit<Props, 'variant'>) {
  const { t } = useTranslation()
  useFileDrop(([path]) => {
    if (path) onDrop(path)
  })

  const pick = () =>
    pickImportFile()
      .then(path => {
        if (path) onDrop(path)
      })
      .catch(() => {})

  const body = (
    <>
      <DownloadGlyph size={16} />
      <div className="min-w-0 text-left">
        <div className="text-sm text-text2">
          {isMobile ? t('Or choose an export file') : t('Or drop an export file here')}
        </div>
        <div className={META_TYPE}>
          {t('csv, json, 1pux — parsed locally, never uploaded')}
        </div>
      </div>
    </>
  )

  const className =
    'flex items-center gap-3.5 rounded-lg border border-dashed border-line2 px-4 py-5 text-text3'

  return isMobile ? (
    <button
      type="button"
      data-testid="import-dropzone"
      onClick={() => void pick()}
      className={`${className} w-full cursor-pointer transition-colors hover:border-accent-line`}
    >
      {body}
    </button>
  ) : (
    <div data-testid="import-dropzone" className={className}>
      {body}
    </div>
  )
}
