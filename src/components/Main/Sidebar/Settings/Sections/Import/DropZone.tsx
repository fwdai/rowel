import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { isMobile } from '@/lib/platform'
import { pickImportFile } from '@/api/pickers'
import { useWebviewDragDrop } from '@/hooks/useWebviewDragDrop'
import { cx } from '@/utils/cx'
import Button from '@/components/elements/Button'
import { CARD } from '@/components/elements/tokens'
import { UploadGlyph } from '../../../../icons'

interface Props {
  onDrop: (path: string) => void
}

// The import section's lead: any export file, with the backend sniffing its
// format. On the desktop a drop target that lights up while a file is held
// over the window, with a picker button as the other way in. Nothing drags a
// file onto a phone, so there the same card is simply the picker — a solid
// card the whole of which is the tap, and no dashed outline promising a
// gesture the platform has not got.
export default function DropZone({ onDrop }: Props) {
  const { t } = useTranslation()
  const [over, setOver] = useState(false)

  useWebviewDragDrop(payload => {
    if (payload.type === 'leave') setOver(false)
    else if (payload.type === 'drop') {
      setOver(false)
      const [path] = payload.paths
      if (path) onDrop(path)
    } else setOver(true)
  })

  const pick = () =>
    pickImportFile()
      .then(path => {
        if (path) onDrop(path)
      })
      .catch(() => {})

  const body = (
    <>
      <div className="grid h-11 w-11 place-items-center rounded-lg bg-accent-soft text-accent">
        <UploadGlyph size={20} />
      </div>
      <div className="mt-3 text-md font-semibold tracking-display text-text">
        {isMobile ? t('Choose an export file') : t('Drop an export file')}
      </div>
      <p className="mt-1 max-w-[360px] text-sm text-text2 max-md:text-base">
        {t('We detect the format automatically. Parsed on this device, never uploaded.')}
      </p>
    </>
  )

  if (isMobile)
    return (
      <button
        type="button"
        data-testid="import-dropzone"
        onClick={() => void pick()}
        className={cx(CARD, 'flex w-full cursor-pointer flex-col items-center p-5 text-center active:bg-hover')}
      >
        {body}
      </button>
    )

  return (
    <div
      data-testid="import-dropzone"
      data-over={over || undefined}
      className={cx(
        'flex flex-col items-center rounded-xl border-[1.5px] border-dashed px-5 py-6 text-center transition-colors',
        over ? 'border-accent-line bg-accent-soft' : 'border-line2 bg-field hover:border-accent-line'
      )}
    >
      {body}
      <Button
        variant="pale"
        size="md"
        className="mt-4"
        onClick={() => void pick()}
        testid="import-choose-file"
      >
        {t('Choose file…')}
      </Button>
    </div>
  )
}
