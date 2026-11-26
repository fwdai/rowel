// Where the photograph goes. The vault holds no photo, so the frame holds the
// silhouette every blank form prints there — shoulders cut by the frame's edge,
// as a passport photo is. It is what makes a face read as an ID at a glance
// rather than as a table of fields.
//
// A fraction of the document's own width, at the 3:4 a portrait photo is shot
// at, so it is the same size on the document however wide the pane is — which
// is how a photograph behaves on a card. Not a fraction of whatever height the
// fields came out at: the picture is part of the document, not a reaction to it.
//
// A little over a fifth, rather than the ~28% a passport and an ID-1 card both
// really use: the number along the foot wants those millimetres more than the
// picture does, and a narrower photo also leaves the fields beside it wide
// enough to set in two lines instead of three. One size on every document.
//
// On a ground of its own, opaque: a photograph is laid over the stock, so a
// card's printed lines stop at its edge rather than running on under the face.
export default function Portrait() {
  return (
    <div
      aria-hidden
      className="flex aspect-[3/4] w-[22%] flex-none items-end justify-center overflow-hidden rounded-[6px] border border-(--face-rule) bg-[#EEEFF2] text-(--face-ink)"
    >
      <svg viewBox="0 0 64 60" className="h-[72%] w-auto opacity-15" fill="currentColor">
        <circle cx="32" cy="17" r="14" />
        <path d="M4 60c0-15 12-26 28-26s28 11 28 26Z" />
      </svg>
    </div>
  )
}
