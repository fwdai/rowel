import type { ReactNode } from 'react'
import Face from '@/components/elements/Face'

interface Props {
  /**
   * The document's real proportions, as `width / height` in millimetres — the
   * shape of the thing in your hand, and the one fixed dimension of a face.
   */
  ratio: string
  /** Which document this is, for the e2e suite and for anyone styling one. */
  docType: string
  /** The colour a card is in, for the same readers. */
  color?: string
  /** `identity-face` for the side a document is read on, `identity-back` for a card's other one. */
  testid: string
  children: ReactNode
}

/**
 * The paper every document face is printed on — the vault's shared `Face`
 * surface, so a passport, a licence and an SSH key's fingerprint plate are one
 * kind of object on the page. What tells one card from another is printed on
 * that paper by its layout (see `Print`), the way it is on the real thing; the
 * stock is the same.
 *
 * The ratio sizes the face, so a document is the shape it really is: a passport
 * page is squarer than a licence because a passport page *is* squarer, and
 * neither is whatever height its contents happened to add up to. A ratio rather
 * than a fixed height, so it is the same object at any pane width.
 *
 * It is carried by a spacer sharing one grid cell with the printing, rather than
 * by `aspect-ratio` on the face itself, and the difference is the whole safety
 * of this. A ratio on the face fixes its height outright, and `overflow-hidden`
 * then swallows anything that does not fit — on a phone-narrow pane the foot of
 * the document went under the rounded corner. (`min-height: auto` does not save
 * it: that only resolves to the content size for a flex or grid *item*, and a
 * face is neither everywhere it is used. Nor does `min-height: fit-content`.)
 * One grid cell holding both takes the taller of the two, which is exactly the
 * rule wanted: the ratio decides the height while the printing fits, and the
 * printing decides it when it does not. In a password manager a value you cannot
 * see is worse than a document half a centimetre too tall — and the layout's job
 * is to keep that the rare case rather than to rely on it.
 */
export default function Frame({ ratio, docType, color, testid, children }: Props) {
  return (
    <Face data-testid={testid} data-doc-type={docType} data-color={color} className="grid w-full">
      <div
        aria-hidden
        className="col-start-1 row-start-1 w-full self-start"
        style={{ aspectRatio: ratio }}
      />
      {/* Positioned and isolated: what a layout pins to the card (its security
          printing, the mark in its corner) is pinned to this, the whole face,
          and the printing can sit under the text without leaving the face. */}
      <div className="relative isolate col-start-1 row-start-1 flex min-w-0 flex-col">
        {children}
      </div>
    </Face>
  )
}
