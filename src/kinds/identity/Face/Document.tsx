import { useState } from 'react'
import Flip from '@/components/elements/Face/Flip'
import type { DocType } from '../templates'
import type { Doc } from './DocCell'
import Frame from './Frame'
import { LAYOUTS } from './layouts'

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
 * nothing; how they are arranged, and what else the card carries, is the
 * layout's.
 *
 * A card has a back and turns over to show it. Only while it is being read:
 * the front is the form, the back has nothing to type, so a card being edited
 * shows its front and offers no turn.
 */
export default function Document({ docType, doc }: Props) {
  const [turned, setTurned] = useState(false)
  const { format, Face, Back } = LAYOUTS[docType]
  const flip = Back && !doc.set ? () => setTurned(!turned) : undefined

  const front = (
    <Frame ratio={format} docType={docType} testid="identity-face">
      <Face docType={docType} doc={doc} flip={flip} />
    </Frame>
  )
  if (!Back) return <div className={WIDTH}>{front}</div>

  return (
    <Flip
      flipped={turned && !!flip}
      className={WIDTH}
      front={front}
      back={
        <Frame ratio={format} docType={docType} testid="identity-back">
          <Back docType={docType} doc={doc} flip={flip} />
        </Frame>
      }
    />
  )
}
