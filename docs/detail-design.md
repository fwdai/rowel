# Entry detail views — the design grammar

How an entry is read, whatever its kind, on the desktop pane and the phone
screen, in both themes. Every detail view is built from three surfaces and
four interactions, and nothing else; a new kind picks from these rather than
inventing its own.

## Three surfaces

| Surface | What it is for | Where |
|---|---|---|
| **Face** | The entry as the physical object it stands for: a credit card, an ID document, an SSH key's fingerprint plate, an API key card. | `elements/Face` |
| **Sealed block** | A body-sized secret that is read rather than taken: a secure note's body, an SSH private key, a raw environment file. | `elements/SealedBlock` |
| **Rows** | The grouped list every value without a shape of its own goes in: a login's credentials, a URL, a date, the user's own fields. | `elements/fields/Row` |

A kind composes these top to bottom: the face first if it has one, then its
rows, then the sealed block, then the note and custom fields in a panel of
their own. A face wider than the pane allows (460px) takes its note panel
beside it on a wide pane and under it elsewhere (`FACE_ASIDE`); the panel is
its own `@container`, so beside a face its rows fold label-over-value.

### Faces

One component draws every face (`Face`), in one of three **tones** — the stock
the object is cut from, defined as `--paper-*`, `--slate-*` and `--plastic-*`
sets in `theme.css`:

- `paper` — an ID document. Fixed light: paper stays paper in the dark theme.
- `plastic` — the credit card. Fixed dark: a card stays black plastic in the
  light theme.
- `slate` — an object of the app rather than of the world (the fingerprint
  plate, the API key card). Paper on the light theme, a slab a shade off the
  pane on the dark one.

Everything printed on a face takes its inks from the tone — `--face-ink`,
`--face-ink2`, `--face-rule`, `--face-hover`, `--face-bad` — never a colour of
its own, so the tones can differ in colour and never in shape. The parts of a
face are shared too: `FaceHeader` (label + chip), `Cell` (caption + value +
copy), `Eye` (the one control), `CopyHint`. Faces pad 20px
(`p-5`), caption at the `CAPTION` tier (10px, tracked), and set their headline
value — a card number, a document number — at `text-xl tracking-secret`, the
same tier a revealed password takes in a row.

An ID document is drawn as the document it is (`kinds/identity/Face/layouts`),
on the same paper. A passport is its data page — ID-3, the machine-readable
band along the foot — and so is a document that is no particular card. A
driving licence, an ID card and a residence permit are each a mock of the card
itself at ID-1, the credit card's shape, and what tells them apart is what
tells the real ones apart:

- **Security printing** (`Print`) — a wash of colour (rose for a licence, blue
  for an ID card, green for a permit) over a fine-line ground. The ground is
  texture, not artwork: it has no figure a real card would have to match.
  Fixed colours, like the paper under them.
- **The mark in the corner** — where a credit card wears its network's: a
  licence's car, an ID card's national emblem as a hologram, a permit's
  fingerprint. Small, in the stock's own ink.
- **The card's own conventions** — the issuing state's plate (`Plate`), a
  licence's fields under the numbers the licence standard gives them, a
  permit's term drawn from issue to expiry (`Term`).

A card has a back and turns over to show it (`elements/Face/Flip`, any face
can use it). What turns it is a small raised key on the card's right edge —
the edge it turns about — outside both sides, so nothing printed on the card
makes room for it and it is in the same place whichever side is up. Under the
pointer it lifts the card a few degrees, which says what it does. The back is
what the card says about itself, drawn from the front and never typed — the
machine-readable zone across the lower third of an ID card and a permit,
a licence's categories, the legend to its field numbers and its barcode — so a
card being edited shows its front and offers no turn. A new document type
gets a layout of its own here rather than a variant of another's.

### Sealed blocks

A panel whose head names the body (`ROW_LABEL`, so on the phone it is the
grouped list's caption), says what it weighs in a mono meta chip (`OPENSSH ·
283 bytes`, `86 chars`), and carries the two controls the body itself cannot
be: a reveal and a copy, plus a kind-local action when the body has one (the
environment file's save). The body is a blurred seal until the eye or the seal
is pressed; revealed, it reads in the primary ink, with a key's armor lines in
the second ink. The editor of a body is a page: a tall box with no underline
(`NoteField` without a label; `PrivateKey` three lines deep).

### Rows

`FieldRow` geometry, unchanged: a label column, the value, a trailing rail. A
login's card is cut into bands (`BAND_GUTTER`) by what the rows are for — the
credentials, where the account lives, the user's own rows — the way a grouped
list is.

## Four interactions

**Copy.** Pressing a value copies it, and the value is the only copy control
there is: a row's value (`CopyValue`), a cell on a face (`Cell`), the card's
number, the API key strip. Under a pointer the value takes a wash and a small
copy glyph at its right edge (`CopyHint`), the same on a row as on a card; on
touch there is no hover and the value is simply pressed. A rail holds only
what means something else — the eye, the open-link, generate, the countdown —
so there is **no copy button** on a row or a face, and the header's primary
action copies the headline secret. The one exception is prose, which is read
rather than pressed: a sealed block's body and the note row (`NoteField`).
Their text can be selected, so a part of it can be taken, and a copy button
takes the whole — in a block's head, beside the eye, and in the note row's
rail.

**Copy feedback.** One behaviour, one component (`elements/Copied`): the
value's glyphs go invisible and a single check takes their place for 1.2s, then
they return. The check is in the `good` ink on a panel and in the surface's own
ink (`--face-ink`) on a face. Never a block painted over the value — that is
what read as a light slab on the dark card and a white check on a white slab in
the dark theme. A control that stays on screen flashes its own check
(`CopyButton`, the primary action); the app-level pill is only for a copy whose
control is gone by the time it lands.

**Reveal.** One eye per face (`Face/Eye`), in the face's own ink, lifting
every mask on it at once; one eye per sealed block, in its head; one eye per
secure row, in its rail. The eye takes a lit state while revealed.

**Mask.** A masked value line is the brand asterisk at a fixed length (`Mask`),
so the mask says nothing about the secret's length — in rows, on the API key
strip, on a document's cells. The credit card alone keeps bullets
(`•••• •••• •••• 4242`): that is how every card in the world is masked, and the
face imitates the card.

## Phone and themes

Nothing branches on the shell. Faces take the screen's width (`max-w-full`);
rows fold by container query; the SSH plate, side by side with its rows on a
pane, stacks over them on the phone. Touch targets grow to 44px through
`TOUCH_TARGET` (controls outside a rail) and a pseudo-element slop on the
face's `Eye`, so a face's layout does not move. The seal's plate says
"tap" where a finger is one of the pointers and "click" otherwise. Themes are
the tones' business: `paper` and `plastic` are fixed, `slate` and every panel
follow `data-theme`.

## Editing

The editor is the read view with inputs in the values' places. Switching
between the two moves nothing: the same shell (`Show/Edit`, a ring inside the
pane rather than a border around it), the same title line (the read `h1` and
the editor's input share `TITLE` and an underline the read one draws
transparent), the same tile (`Show/Identity`'s `Tile`, reading a `Look` off
the draft so the favicon, brand mark or passport cover is there as the fields
are typed), and the same surfaces:

- **Rows** swap `CopyValue` for the input under the same label, as
  `fields/Field` always has. The one-time code proves its secret with the
  read row's countdown ring in the rail, not a line under the box. A row may
  offer values back as it is typed into (`Field`'s `suggestions`, drawn by
  `elements/Suggestions`): a list dropped out of the box on the `POPOVER`
  ground — the menu's quieter cousin, solid and barely lifted — starting
  where the box starts, just under its underline, and as wide as its longest
  option. One line each: the value, then how many items hold it. A login's
  username and email offer the usernames the lockbox already holds, most
  used first — list metadata, so nothing is revealed to build them. The list answers typing:
  an empty box is offered nothing, and focus alone opens nothing. The caret
  never leaves the box and nothing is lit until an arrow or the pointer
  lights it, so Enter and Tab keep their meaning.
- **Faces** are the form. `Face/Cell` takes a writer (`onChange`) and is then
  the input in its place, caption and all, in the face's inks; a derived
  value (the SSH plate, an API key's host) stays printed. A cell's `size` is
  its width in characters in both modes, so the cells flow the same way
  whichever mode the face is in. The eye is the same control, lit while the
  editor starts revealed.
- **Sealed blocks** take a writer and become a page to type on under the same
  head, the eye hiding the typing, the copy standing down; the generator
  sits in the head of the SSH key's block.

A kind composes its editor from the same parts in the same order as its read
view, so there is one `Fields` per kind and no `Read` beside it. Reading, an
empty panel is left out; editing, every panel is there to be filled.

## The phone

Nothing on the phone is a second implementation of the above. The compact
shell (`Main/Compact`) chooses where the shared parts go — a nav row over one
scroller, the primary action at the bottom edge, the footer's facts as
sections — and the parts themselves adapt by CSS: rows fold by container
query, faces take the screen's width, touch targets grow through
`any-pointer-coarse`. The read screen and the form screen draw the same
header (tile, title, the kind's own eyebrow under it) from the entry or the
draft, so stepping into edit changes the nav row and nothing else. The nav
row's worded controls are one `Compact/NavButton`.
