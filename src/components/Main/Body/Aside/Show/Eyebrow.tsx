import { useTranslation } from 'react-i18next'
import type { Entry, EntryType } from '@/api/types'
import type { EntryDraft } from '@/kinds/draft'
import { kindOf } from '@/kinds'
import { LABEL } from '@/components/elements/tokens'

interface Props {
  /** What the entry is, and the host its metadata carries. */
  entry: { type: EntryType; urlHost?: string }
  /**
   * The decrypted entry — or the draft being written, which is the same
   * shape — or null while `revealEntry` is still in flight.
   */
  revealed: Entry | EntryDraft | null
  /** How the line sits in the header the shell built around it. */
  className?: string
  /**
   * Only what the kind has to say about this entry — a document's type — and
   * nothing when it has nothing: no kind name, no host. The phone header takes
   * this: its rows already say what the entry is and where it belongs, and a
   * line repeating both under the title was two lines of header for one line
   * of information.
   */
  bare?: boolean
}

// What the entry is, in one micro line: the kind, and — once it can say so — the
// host it belongs to, or, for a document whose type is encrypted, what kind of
// document it is. Both shells draw it, reading and writing; only its place in
// the header differs.
export default function Eyebrow({ entry, revealed, className, bare }: Props) {
  const { t } = useTranslation()
  const kind = kindOf(entry.type)
  // Whatever the kind can say about this entry — an identity's document type,
  // once the payload is in hand — and the host otherwise, which is metadata and
  // needs no reveal.
  // Spread, as the field set is handed the entry: a kind's interface has no
  // index signature, and the draft shape the kind reads does.
  const fromKind = revealed ? kind.eyebrow?.({ ...revealed }) : null
  const segment: { text: string; testid?: string } | null =
    fromKind ?? (!bare && entry.urlHost ? { text: entry.urlHost } : null)

  if (bare && !segment) return null

  return (
    // On the phone it sits under the title as a plain 13px line rather than a
    // tracked uppercase label — the prototype's "aws.amazon.com · Work".
    <div className={`${className} ${LABEL} max-md:text-base max-md:normal-case max-md:tracking-[0]`}>
      {!bare && <span className="text-text2">{t(kind.label)}</span>}
      {segment && (
        <>
          {!bare && <span>·</span>}
          <span data-testid={segment.testid}>{segment.text}</span>
        </>
      )}
    </div>
  )
}
