import { useEffect, useRef, type KeyboardEvent } from 'react'
import { useTranslation } from 'react-i18next'
import type { EntryType } from '@/api/types'
import { useUi, closeAddPicker, startEntry } from '@/store'
import { KINDS } from '@/kinds'
import Frame from '@/components/elements/Frame'
import DialogHeader from '@/components/elements/DialogHeader'
import { useFrameOwnsClose } from '@/components/elements/frameContext'
import { CARD, LABEL } from '@/components/elements/tokens'
import { cx } from '@/utils/cx'
import KindTile from './KindTile'
import ScanAction from './ScanAction'
import EnvAction from './EnvAction'
import ReceiveAction from './ReceiveAction'
import GenerateAction from './GenerateAction'

const TITLE_ID = 'add-secret-title'

const COLUMNS = 3

// How far each arrow moves through the 3-column grid. `useRadioNav` is the 1-D
// radiogroup pattern (selection follows focus, one tab stop), which is the
// wrong contract here: these tiles are actions, so focus has to move without
// choosing a kind.
const STEP: Record<string, number> = {
  ArrowRight: 1,
  ArrowLeft: -1,
  ArrowDown: COLUMNS,
  ArrowUp: -COLUMNS
}

// The "Add a secret" kind picker: the one place the app asks *what* you are
// saving. Mounted once from Main; renders nothing until `ui.addPicker`.
export default function AddSecret() {
  const { t } = useTranslation()
  const open = useUi(state => state.addPicker)
  const grid = useRef<HTMLDivElement>(null)
  const sheet = useFrameOwnsClose()

  const tiles = () => Array.from(grid.current?.querySelectorAll('button') ?? [])

  // The picker exists to be answered from the keyboard, so the first choice is
  // focused the moment it opens.
  useEffect(() => {
    if (open) grid.current?.querySelector('button')?.focus()
  }, [open])

  if (!open) return null

  const pick = (type: EntryType) => {
    startEntry(type)
    closeAddPicker()
  }

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const buttons = tiles()
    if (buttons.length === 0) return

    // 1..n picks the nth kind outright — the fastest path through the modal.
    const digit = Number(event.key)
    if (digit >= 1 && digit <= buttons.length) {
      buttons[digit - 1].click()
      event.preventDefault()
      return
    }

    const step = STEP[event.key]
    if (step === undefined) return
    const from = Math.max(buttons.indexOf(document.activeElement as HTMLButtonElement), 0)
    buttons[(from + step + buttons.length) % buttons.length].focus()
    event.preventDefault()
  }

  // The header carries the close, so the frame draws none. The gutters come off
  // the frame's own width rather than the shell's: 28px is a tenth of the card
  // and a fifteenth of a phone.
  //
  // `fit="content"`: a header, a 3-column grid and a short list are little
  // enough to answer from the bottom edge, so the phone frames this as a bottom
  // sheet rather than a page.
  return (
    <Frame
      onClose={closeAddPicker}
      labelledBy={TITLE_ID}
      testid="add-secret-modal"
      fit="content"
      align="center"
      hideClose
      className="flex max-h-[80vh] w-dialog"
    >
      {/* The settings ground on the desktop card. A sheet keeps its own white:
          it pads under its children, and the grey would stop short of that. */}
      <div className={cx('@container flex w-full flex-col', !sheet && 'bg-pane')}>
        <DialogHeader
          id={TITLE_ID}
          title={t('Add a secret')}
          description={t('Everything is encrypted before it touches disk.')}
          onClose={closeAddPicker}
          className="px-7 @max-[500px]:px-5"
        />
        <div className="min-h-0 overflow-y-auto p-7 @max-[500px]:p-5">
          <div ref={grid} onKeyDown={onKeyDown} className="grid grid-cols-3 gap-2.5">
            {KINDS.map((kind, index) => (
              <KindTile
                key={kind.type}
                kind={kind}
                digit={index + 1}
                onSelect={() => pick(kind.type)}
              />
            ))}
          </div>

          <h2 className={cx(LABEL, 'mt-6 mb-2')}>{t('Other ways in')}</h2>
          <div className={CARD}>
            <ScanAction />
            <EnvAction />
            <ReceiveAction />
            <GenerateAction />
          </div>
        </div>
      </div>
    </Frame>
  )
}
