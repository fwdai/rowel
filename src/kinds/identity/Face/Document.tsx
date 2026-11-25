import { useState, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import Flip from '@/components/elements/Face/Flip'
import Segmented from '@/components/elements/Segmented'
import type { DocType } from '../templates'
import type { Doc } from './DocCell'
import Frame from './Frame'
import { LAYOUTS } from './layouts'
import Card from './layouts/Card'

interface Props {
  docType: DocType
  doc: Doc
}

type Side = 'front' | 'back'

// A document is 460px wide where the pane allows it (see `FACE_ASIDE`) and the
// pane's width where it does not.
const WIDTH = 'w-[460px] max-w-full'

/**
 * An identity document as an object: the paper it is on, cut to the document's
 * real shape, and on it the layout that document has (see `layouts`). Which
 * rows appear is the template's business and a cell with nothing in it prints
 * nothing; how they are arranged is the layout's.
 *
 * A card has a back, and a switch under it says which side is up. The switch
 * stands outside the card, so nothing printed on the card has to make room for
 * it and it stays where it is while the card turns. Only while the card is
 * being read: the front is the form, the back has nothing to type, so a card
 * being edited shows its front and offers no switch.
 */
export default function Document({ docType, doc }: Props) {
  const { t } = useTranslation()
  const [side, setSide] = useState<Side>('front')
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
    <div className={WIDTH}>
      <Flip
        flipped={side === 'back' && !editing}
        front={face('identity-face', front)}
        back={face('identity-back', <card.Back docType={docType} doc={doc} stock={card.stock} />, true)}
      />
      {!editing && (
        <div className="mt-3 flex justify-center">
          <Segmented
            name={t('Flip card')}
            options={[
              { value: 'front', label: t('Front side') },
              { value: 'back', label: t('Back side') }
            ]}
            value={side}
            onChange={setSide}
            testidPrefix="card-side"
          />
        </div>
      )}
    </div>
  )
}
