import { useState, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import Flip from '@/components/elements/Face/Flip'
import type { DocType } from '../templates'
import ColorSwatches from './ColorSwatches'
import type { Doc } from './DocCell'
import Frame from './Frame'
import type { CardColor } from './colors'
import { LAYOUTS, cardColorFor } from './layouts'
import Card from './layouts/Card'

interface Props {
  docType: DocType
  doc: Doc
  /** The card colour as the entry stores it; anything not a palette key reads as the type's own. */
  color?: string
  /** Stores a picked colour; absent while reading. */
  onColor?: (color: CardColor) => void
}

// A document is 460px wide where the pane allows it (see `FACE_ASIDE`) and the
// pane's width where it does not.
const WIDTH = 'w-[460px] max-w-full'

// A card's mark, in px: in the corner of its front, and in the stock of its back.
const MARK = 28
const WATERMARK = 88

/**
 * An identity document as an object: the paper it is on, cut to the document's
 * real shape, and on it the layout that document has (see `layouts`). Which
 * rows appear is the template's business and a cell with nothing in it prints
 * nothing; how they are arranged is the layout's.
 *
 * A card has a back and turns over to show it (see `Flip`). Only while it is
 * being read: the front is the form, the back has nothing to type, so a card
 * being edited shows its front and offers no turn — and offers its colours
 * under it instead.
 */
export default function Document({ docType, doc, color, onColor }: Props) {
  const { t } = useTranslation()
  const [turned, setTurned] = useState(false)
  const { format, Face, card } = LAYOUTS[docType]
  const front = <Face docType={docType} doc={doc} />

  if (!card)
    return (
      <div className={WIDTH}>
        <Frame ratio={format} docType={docType} testid="identity-face">
          {front}
        </Frame>
      </div>
    )

  const editing = !!doc.set
  const worn = cardColorFor(docType, color) ?? card.color
  const face = (testid: string, content: ReactNode, back?: boolean) => (
    <Frame ratio={format} docType={docType} color={worn} testid={testid}>
      <Card color={worn} mark={card.mark(MARK, doc)} back={back}>
        {content}
      </Card>
    </Frame>
  )

  return (
    <div className={WIDTH}>
      <Flip
        flipped={turned && !editing}
        onFlip={editing ? undefined : () => setTurned(!turned)}
        label={t('Flip card')}
        front={face('identity-face', front)}
        back={face(
          'identity-back',
          <card.Back
            docType={docType}
            doc={doc}
            watermark={
              <div aria-hidden className="flex-none text-(--card-line) opacity-30">
                {card.mark(WATERMARK, doc)}
              </div>
            }
          />,
          true
        )}
      />
      {onColor && <ColorSwatches value={worn} onChange={onColor} />}
    </div>
  )
}
