import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { cardBrandOf, cardDigits, groupCardNumber, hasBrandMark } from '@/utils/cardBrand'
import CardBrandMark from '@/components/elements/CardBrandMark'
import Face from '@/components/elements/Face'
import Cell, { CAPTION } from '@/components/elements/Face/Cell'
import Eye from '@/components/elements/Face/Eye'
import Panel from '@/components/elements/Panel'
import { FACE_ASIDE } from '@/components/elements/tokens'
import {
  CustomFieldsField,
  NoteField,
  hasExtra,
  useField,
  useFields
} from '@/components/elements/fields'
import { CARD_MASK } from '../meta'
import { formatExpiry, isExpired, splitExpiry } from '../expiry'

// The card, with what the plastic has no room for — the note and the user's
// own rows — on a panel beside it. In both modes: the face IS the form, each
// printed value an input in the same place (see `Face/Cell`).
export default function Fields() {
  const { t } = useTranslation()
  const { entry, set, attempted } = useFields()
  const editing = !!set
  // Editing starts revealed, since typing into dots is guesswork; the same
  // eye takes it back in either mode.
  const [show, setShow] = useState(editing)

  const name = useField('name')
  const number = useField('number')
  const cvc = useField('cvc')
  const pin = useField('pin')
  const month = useField('month')
  const year = useField('year')
  const note = useField('note')

  // The card is 460 wide whatever else there is, so the note and the custom
  // rows go in a column of their own — and reading, an empty column is no
  // column at all.
  const aside = editing || !!note.value || hasExtra(entry)
  const expired = !editing && isExpired(month.value, year.value)

  // Derived live from the number in hand, so it tracks an unsaved edit too.
  const brand = cardBrandOf(number.value)
  const grouped = groupCardNumber(number.value)
  const expiry = formatExpiry(month.value, year.value)
  const setExpiry = (typed: string) => {
    const parts = splitExpiry(typed)
    month.set(parts.month)
    year.set(parts.year)
  }

  return (
    <div className={aside ? FACE_ASIDE : undefined}>
      {/* The vault's shared face (see elements/Face) in its plastic tone: black
          plastic on both palettes, every ink and rule on it the tone's own. The
          number's wide tracking and the bullet masks imitate a real card, so
          those alone stay the card's. */}
      {/* A real card's proportions — ISO 7810 ID-1, 85.60 × 53.98 mm — from the
          width, whatever the width is: 460px on the desktop, the screen's
          gutters on a phone. A fixed height gave a squat 1.24:1 slab wherever
          the width was squeezed. The height is a floor, not a cap: with more
          rows than the face has room for, it grows rather than clipping. */}
      <Face
        tone="plastic"
        data-testid="card-face"
        className="flex aspect-[1.586] w-[460px] max-w-full flex-col p-5"
      >
        <div className="flex items-start justify-between gap-4">
          <Cell
            name="name"
            display={name.value || t('Card')}
            value={name.value}
            onChange={name.set}
            placeholder={t('Cardholder')}
            maxLength={40}
            ink="text-base uppercase tracking-label"
            className="flex-1"
          />
          {hasBrandMark(brand) ? (
            <CardBrandMark brand={brand} size={22} tone="light" />
          ) : (
            <div className="h-7 w-[40px] flex-none rounded-xs border border-(--face-rule) bg-(--face-hover)" />
          )}
        </div>

        <div className="flex-1" />

        <Cell
          name="number"
          display={
            number.value
              ? show
                ? grouped
                : `•••• •••• •••• ${cardDigits(number.value).slice(-4)}`
              : CARD_MASK
          }
          value={number.value}
          box={grouped}
          onChange={next => number.set(cardDigits(next))}
          // Mask, not real catalog copy — it has no translation.
          placeholder={CARD_MASK}
          required
          maxLength={23}
          masked={!show}
          // The headline-secret tier every face's number takes (a document's,
          // a revealed password's): 20px, tracked for reading aloud.
          ink="text-xl tracking-secret"
          className="self-stretch"
        />

        {/* Three equal columns on the number's width, plus the eye's slot. */}
        <div className="mt-5 grid grid-cols-[repeat(3,minmax(0,1fr))_28px] items-end gap-3.5">
          <Cell
            name="expiry"
            label="Expires"
            display={expiry || '••/••'}
            value={expiry}
            onChange={setExpiry}
            placeholder={t('MM/YY')}
            required
            // One box, two draft keys: "12" fills the box but not the pair,
            // and `isValid` wants both — so the box has to say so itself.
            invalid={attempted && !(month.value && year.value)}
            maxLength={5}
            ink="text-base"
            testid="entry-value-expires"
            flag={
              expired && (
                <span className={`flex-none ${CAPTION} text-(--face-bad)`}>{t('Expired')}</span>
              )
            }
          />
          <Cell
            name="cvc"
            label="CVC"
            display={show ? cvc.value : '•••'}
            value={cvc.value}
            onChange={next => cvc.set(cardDigits(next))}
            placeholder="•••"
            required
            maxLength={4}
            masked={!show}
            ink="text-base"
          />
          <Cell
            name="pin"
            label="Pin"
            display={show ? pin.value : '••••'}
            value={pin.value}
            onChange={next => pin.set(cardDigits(next))}
            placeholder="••••"
            maxLength={6}
            masked={!show}
            ink="text-base"
          />
          {/* The face's one control, where the card's masks are. */}
          <Eye
            shown={show}
            onToggle={() => setShow(!show)}
            testid="card-reveal-button"
            className="justify-self-end"
          />
        </div>
      </Face>

      {aside && (
        // Its own container: beside a 460px face the column is under the
        // 420px the label column needs, so its rows fold label-over-value
        // (see fields/Row) rather than squeezing a sentence into a sliver.
        <Panel className="@container">
          <NoteField label="Note" />
          <CustomFieldsField />
        </Panel>
      )}
    </div>
  )
}
