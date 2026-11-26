import type { DocType } from '../templates'
import ColorSwatches from './ColorSwatches'
import type { Doc } from './DocCell'
import Frame from './Frame'
import { cardColorOf, type CardColor } from './colors'
import { LAYOUTS } from './layouts'
import Card from './layouts/Card'

interface Props {
  docType: DocType
  doc: Doc
  /** The card colour as the entry stores it; anything not a palette key reads as the type's own. */
  color?: string
  /** Stores a picked colour; absent while reading. */
  onColor?: (color: CardColor) => void
}

/**
 * An identity document as an object: the paper it is on, cut to the document's
 * real shape, and on it the layout that document has (see `layouts`). Which
 * rows appear is the template's business and a cell with nothing in it prints
 * nothing; how they are arranged is the layout's.
 *
 * A card is printed in its colour, and while it is being edited offers its
 * colours under it.
 *
 * 460px wide where the pane allows it (see `FACE_ASIDE`) and the pane's width
 * where it does not.
 */
export default function Document({ docType, doc, color, onColor }: Props) {
  const { format, Face, card } = LAYOUTS[docType]
  const worn = card && (cardColorOf(color) ?? card.color)
  const face = <Face docType={docType} doc={doc} />

  return (
    <div className="w-[460px] max-w-full">
      <Frame ratio={format} docType={docType} color={worn}>
        {card && worn ? (
          <Card color={worn} mark={card.mark(doc)}>
            {face}
          </Card>
        ) : (
          face
        )}
      </Frame>
      {worn && onColor && <ColorSwatches value={worn} onChange={onColor} />}
    </div>
  )
}
