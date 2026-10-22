# Entry detail views — the design grammar

How an entry is read, whatever its kind, on the desktop pane and the phone
screen, in both themes. Every detail view is built from three surfaces and
four interactions, and nothing else; a new kind picks from these rather than
inventing its own.

## Three surfaces

| Surface | What it is for | Where |
|---|---|---|
| **Face** | The entry as the physical object it stands for: a credit card, an ID document, an SSH key's fingerprint plate, an API key card. | `elements/Face` |
| **Sealed block** | A body-sized secret that is read rather than taken: a secure note's body, an SSH private key. | `elements/SealedBlock` |
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

### Sealed blocks

A panel whose head names the body (`ROW_LABEL`, so on the phone it is the
grouped list's caption), says what it weighs in a mono meta chip (`OPENSSH ·
283 bytes`, `86 chars`), and carries the two controls the body itself cannot
be: a reveal and a copy. The body is a blurred seal until the eye or the seal
is pressed; revealed, it reads in the primary ink, with a key's armor lines in
the second ink. The editor of a body is a page: a tall box with no underline
(`NoteField` without a label; `PrivateKey` three lines deep).

### Rows

`FieldRow` geometry, unchanged: a label column, the value, a trailing rail. A
login's card is cut into bands (`BAND_GUTTER`) by what the rows are for — the
credentials, where the account lives, the user's own rows — the way a grouped
list is.

## Four interactions

**Copy.** Pressing a value copies it, everywhere: a row's value
(`CopyValue`), a cell on a face (`Cell`), the card's number, the API key
strip. On a pointer device a row also shows its copy button in the rail on
hover (a secure row keeps it in sight, since it has a rail already), and a face
shows a small copy glyph in the cell's corner (`CopyHint`). Faces carry **no
persistent copy button**: their one control is the eye, and the header's
primary action copies the headline secret. Sealed blocks, whose body cannot be
pressed to copy, carry the copy in their head. On the phone, copy buttons are
hidden (`ROW_COPY`) and the value is the whole control.

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
