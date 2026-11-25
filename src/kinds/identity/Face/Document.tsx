import { useState, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import Flip from '@/components/elements/Face/Flip'
import type { DocType } from '../templates'
import type { Doc } from './DocCell'
import Frame from './Frame'
import { LAYOUTS } from './layouts'
import Card from './layouts/Card'

interface Props {
  docType: DocType
  doc: Doc
}

// A document is 460px wide where the pane allows it (see `FACE_ASIDE`) and the
// pane's width where it does not.
const WIDTH = 'w-[460px] max-w-full'

/**
 * An identity document as an object: the paper it is on, cut to the document's
 * real shape, and on it the layout that document has (see `layouts`). Which
 * rows appear is the template's business and a cell with nothing in it prints
 * nothing; how they are arranged is the layout's.
 *
 * A card has a back and turns over to show it (see `Flip`). Only while it is
 * being read: the front is the form, the back has nothing to type, so a card
 * being edited shows its front and offers no turn.
 */
export default function Document({ docType, doc }: Props) {
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
  const face = (testid: string, content: ReactNode, back?: boolean) => (
    <Frame ratio={format} docType={docType} testid={testid}>
      <Card stock={card.stock} doc={doc} back={back}>
        {content}
      </Card>
    </Frame>
  )

  return (
    <Flip
      className={WIDTH}
      flipped={turned && !editing}
      onFlip={editing ? undefined : () => setTurned(!turned)}
      label={t('Flip card')}
      front={face('identity-face', front)}
      back={face('identity-back', <card.Back docType={docType} doc={doc} stock={card.stock} />, true)}
    />
  )
}
