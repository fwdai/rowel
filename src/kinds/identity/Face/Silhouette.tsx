/**
 * The head and shoulders every blank form prints where the photograph goes, as
 * two shapes on a 64 × 60 grid — for the `<svg>` that draws them to size, ink
 * and place: the portrait's frame, or the ghost image in a card's stock.
 */
export default function Silhouette() {
  return (
    <>
      <circle cx="32" cy="17" r="14" />
      <path d="M4 60c0-15 12-26 28-26s28 11 28 26Z" />
    </>
  )
}
