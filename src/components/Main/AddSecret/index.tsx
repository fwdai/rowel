import { useEffect, useRef, useState, type KeyboardEvent } from 'react'
import { useTranslation } from 'react-i18next'
import type { EntryType } from '@/api/types'
import { useUi, closeAddPicker, setAddPickerKind, startEntry } from '@/store'
import { KINDS, kindOf, type Kind } from '@/kinds'
import Frame from '@/components/elements/Frame'
import DialogHeader from '@/components/elements/DialogHeader'
import IconButton from '@/components/elements/IconButton'
import { useFrameOwnsClose } from '@/components/elements/frameContext'
import { CARD, LABEL } from '@/components/elements/tokens'
import { BackGlyph } from '@/components/Main/icons'
import { cx } from '@/utils/cx'
import KindTile from './KindTile'
import ChoiceRow from './ChoiceRow'
import ScanAction from './ScanAction'
import ReceiveAction from './ReceiveAction'

const TITLE_ID = 'add-secret-title'

const COLUMNS = 3

// How far each arrow moves through a page of `columns` buttons: the kinds'
// 3-column grid, or a kind's one-column list of answers. `useRadioNav` is the
// 1-D radiogroup pattern (selection follows focus, one tab stop), which is the
// wrong contract here: these are actions, so focus has to move without
// choosing anything.
const stepOf = (key: string, columns: number): number | undefined =>
  ({ ArrowRight: 1, ArrowLeft: -1, ArrowDown: columns, ArrowUp: -columns })[key]

/**
 * The "Add a secret" picker: the one place the app asks *what* you are saving.
 * Mounted once from Main; renders nothing until `ui.addPicker`.
 *
 * Two pages in one frame. The first is the kinds; a kind with a question of
 * its own (`Kind.choices` — which document an identity is) turns the page to
 * its answers rather than opening its form, so the form arrives already cut
 * to the answer and never changes shape once it is on screen. The page turns
 * the way a settings sub-page steps in and out, under a Back control; the
 * column keeps the height it had, so a shorter page never drops the sheet's
 * top edge or shrinks the card under the pointer.
 */
export default function AddSecret() {
  const { t } = useTranslation()
  const open = useUi(state => state.addPicker)
  const asking = useUi(state => state.addPickerKind)
  const kind = asking ? kindOf(asking) : null
  // The question on screen, when the picker is past the kinds.
  const question = kind?.choices ? { kind, choices: kind.choices } : null
  const column = useRef<HTMLDivElement>(null)
  const page = useRef<HTMLDivElement>(null)
  // The kind whose page was just left, so its tile takes the focus back.
  const left = useRef<EntryType | null>(null)
  const sheet = useFrameOwnsClose()
  // Which way the page last turned, for its arrival. Nothing for the page the
  // picker opens on: the frame's own arrival is the motion there.
  const [turn, setTurn] = useState<'forward' | 'back' | null>(null)
  // The column's height as the page turned forward, kept as its floor.
  const [floor, setFloor] = useState<number>()
  // How the page on screen is laid out, for the arrow keys.
  const columns = question ? 1 : COLUMNS

  // The picker exists to be answered from the keyboard, so each page's first
  // choice is focused as it arrives — or, back on the kinds, the tile that was
  // stepped into.
  useEffect(() => {
    if (!open) return
    const from = left.current
    left.current = null
    const tile =
      from && page.current?.querySelector<HTMLButtonElement>(`[data-testid="add-kind-${from}"]`)
    ;(tile || page.current?.querySelector('button'))?.focus()
  }, [open, asking])

  // Closed, the next opening starts afresh: no page turn, no floor.
  useEffect(() => {
    if (open) return
    setTurn(null)
    setFloor(undefined)
    left.current = null
  }, [open])

  if (!open) return null

  const start = (type: EntryType, answer?: Record<string, string>) => {
    startEntry(type, answer)
    closeAddPicker()
  }

  const pick = (next: Kind) => {
    if (!next.choices) return start(next.type)
    setFloor(column.current?.offsetHeight)
    setTurn('forward')
    setAddPickerKind(next.type)
  }

  const back = () => {
    if (!kind) return
    left.current = kind.type
    setTurn('back')
    setAddPickerKind(null)
  }

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const items = Array.from(page.current?.querySelectorAll('button') ?? [])
    if (items.length === 0) return

    // 1..n picks the nth choice outright — the fastest path through the modal.
    const digit = Number(event.key)
    if (digit >= 1 && digit <= items.length) {
      items[digit - 1].click()
      event.preventDefault()
      return
    }

    const step = stepOf(event.key, columns)
    if (step === undefined) return
    const from = Math.max(items.indexOf(document.activeElement as HTMLButtonElement), 0)
    items[(from + step + items.length) % items.length].focus()
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
      <div
        ref={column}
        style={{ minHeight: floor }}
        className={cx('@container flex w-full flex-col', !sheet && 'bg-pane')}
      >
        {/* The question replaces the title rather than stacking under it, and
            the Back control sits where the settings sub-pages put theirs. */}
        <DialogHeader
          id={TITLE_ID}
          title={question ? t(question.choices.title) : t('Add a secret')}
          description={question ? undefined : t('Everything is encrypted before it touches disk.')}
          onClose={closeAddPicker}
          leading={
            question && (
              <IconButton
                muted
                label={t('Back')}
                title={t('Back')}
                testid="add-secret-back"
                onClick={back}
                className="self-center"
              >
                <BackGlyph size={16} />
              </IconButton>
            )
          }
          className="px-7 @max-[500px]:px-5"
        />
        {/* Keyed by page, so a turn mounts the new page and plays its step. */}
        <div
          key={question ? question.kind.type : 'kinds'}
          className={cx(
            'min-h-0 overflow-y-auto p-7 @max-[500px]:p-5',
            turn === 'forward' && 'animate-step-forward',
            turn === 'back' && 'animate-step-back'
          )}
        >
          {question ? (
            <div ref={page} onKeyDown={onKeyDown} className={CARD}>
              {question.choices.options.map((option, index) => (
                <ChoiceRow
                  key={option.value}
                  option={option}
                  tint={question.kind.tint}
                  digit={index + 1}
                  onSelect={() =>
                    start(question.kind.type, { [question.choices.field]: option.value })
                  }
                />
              ))}
            </div>
          ) : (
            <>
              <div ref={page} onKeyDown={onKeyDown} className="grid grid-cols-3 gap-2.5">
                {KINDS.map((next, index) => (
                  <KindTile
                    key={next.type}
                    kind={next}
                    digit={index + 1}
                    onSelect={() => pick(next)}
                  />
                ))}
              </div>

              {/* Only what no tile covers: the env tile's editor takes a dropped
                  or picked file itself, and a generated password belongs to the
                  login it is for. */}
              <h2 className={cx(LABEL, 'mt-6 mb-2')}>{t('Other ways in')}</h2>
              <div className={CARD}>
                <ScanAction />
                <ReceiveAction />
              </div>
            </>
          )}
        </div>
      </div>
    </Frame>
  )
}
