import { useTranslation } from 'react-i18next'
import { pickBackup } from '@/api/pickers'
import { useFileDrop } from '@/hooks/useFileDrop'
import { isRestorableFile } from '@/lib/backup'
import { DownloadGlyph } from '@/components/Main/icons'
import { META } from '@/components/elements/tokens'

interface Props {
  onPick: (path: string) => void
}

// A backup is a file, so the target is the file's own shape: drop it anywhere
// on the window, or press to open the picker. Desktop only — the screen that
// draws it is not offered on a phone, where nothing drags a file anywhere.
export default function DropZone({ onPick }: Props) {
  const { t } = useTranslation()

  useFileDrop(paths => {
    const path = paths.find(isRestorableFile)
    if (path) onPick(path)
  })

  const pick = () =>
    pickBackup()
      .then(path => {
        if (path) onPick(path)
      })
      .catch(() => {})

  return (
    <button
      type="button"
      data-testid="restore-dropzone"
      onClick={() => void pick()}
      className="flex w-full cursor-pointer flex-col items-center justify-center gap-2.5 rounded-xl border border-dashed border-line2 bg-field px-4 py-10 transition-colors hover:border-accent-line hover:bg-accent-soft"
    >
      <span className="text-text2">
        <DownloadGlyph size={24} />
      </span>
      <span className="text-base font-medium text-text">
        {t('Drop a .rowel or .swftx file here')}
      </span>
      <span className={META}>
        <span className="text-accent">{t('or browse…')}</span>
      </span>
    </button>
  )
}
