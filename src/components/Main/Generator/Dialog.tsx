import { useRef } from 'react'
import { useTranslation } from 'react-i18next'
import IconButton from '@/components/elements/IconButton'
import Frame from '@/components/elements/Frame'
import { useFrameOwnsClose } from '@/components/elements/frameContext'
import type { GeneratorApply, SshApply } from '@/store'
import { RefreshGlyph } from '../icons'
import { useGeneratorDialog } from './useGeneratorDialog'
import { useDialogKeys } from './useDialogKeys'
import Actions from './Actions'
import Card from './Card'
import Tabs from './Tabs'
import Panel from './Panel'

interface Props {
  apply: GeneratorApply | null
  ssh: SshApply | null
  onClose: () => void
}

// The generator as an overlay. On the desktop it is a 560px settings-style card
// (`Card`); in a phone's sheet, opened from a password row, it keeps the compact
// stack the phone's tab root also draws. ⏎ confirms, Esc closes.
export default function Dialog({ apply, ssh, onClose }: Props) {
  const { t } = useTranslation()
  const generator = useGeneratorDialog(apply, ssh, onClose)
  const { mode, setMode, regenerate, confirm } = generator
  const cardRef = useRef<HTMLDivElement>(null)
  useDialogKeys(cardRef, confirm, onClose)
  const phone = useFrameOwnsClose()

  // The card carries its own title row and its own Cancel, so the frame adds no
  // chrome of its own. It takes `cardRef` too, so the topmost-dialog check
  // still finds itself. The sheet ignores `className`.
  return (
    <Frame
      ref={cardRef}
      onClose={onClose}
      labelledBy="generator-title"
      testid="generator-dialog"
      className="flex max-h-[calc(100vh-56px)] w-dialog-sm max-w-[calc(100vw-56px)] flex-col bg-pane"
      align="center"
      hideClose
    >
      {phone ? (
        <>
          <div className="flex items-center gap-2.5 px-[18px] py-[15px] inset-shadow-hairline">
            <div id="generator-title" className="flex-1 text-lg font-semibold tracking-display">
              {t('Generate')}
            </div>
            {/* Opened for a key, there is nothing to switch to. */}
            {!ssh && <Tabs mode={mode} ssh={!apply} onChange={setMode} />}
          </div>
          <div className="p-[18px]">
            <Panel generator={generator} />
            <div className="mt-[18px] flex items-center gap-1.5">
              <IconButton
                title={t('Regenerate')}
                testid="generator-regenerate"
                onClick={regenerate}
                className="h-9 w-9 border border-line2 hover:border-accent-line"
              >
                <RefreshGlyph />
              </IconButton>
              <div className="flex-1" />
              <Actions generator={generator} onClose={onClose} />
            </div>
          </div>
        </>
      ) : (
        <Card generator={generator} apply={apply} ssh={ssh} onClose={onClose} />
      )}
    </Frame>
  )
}
