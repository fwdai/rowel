import type { ComponentType } from 'react'
import type { TKey } from '@/i18n'
import type { Entry, EntryMeta, EntryType } from '@/api/types'
import type { EntryDraft } from '@/kinds/draft'
import type { LoginGlyph } from '@/components/Main/icons'
import type { ContentProps } from '@/components/Main/Body/List/Item/Row'

// Every icon in the set shares this signature (see Main/icons.tsx).
export type Glyph = typeof LoginGlyph

/**
 * Everything the app needs to know about one kind of secret, in one object.
 *
 * Adding a kind means writing one of these and listing it in `KINDS` — no
 * switch statement, glyph map or label record anywhere else has to learn
 * about it.
 */
export interface Kind {
  type: EntryType
  /** Singular, untranslated — call sites wrap it in `t()`. */
  label: TKey
  /** Plural, untranslated (list-column title, scope menu, section caption). */
  pluralLabel: TKey
  /** One line for pickers and empty states. */
  description: TKey
  /**
   * Whole sentences rather than a template plus a spliced noun. Languages that
   * inflect (ru, uk, pl) cannot take a nominative noun dropped into a slot, so
   * each kind names its own phrasing and the translator gets a full sentence.
   */
  addLabel: TKey
  untitledLabel: TKey
  emptyLabel: TKey
  noMatchesLabel: TKey
  Glyph: Glyph
  /** Key into the `--color-kind-*` tokens (see styles/theme.css). */
  tint: 'login' | 'card' | 'note' | 'identity' | 'ssh' | 'env' | 'apikey'
  /** The empty draft a new entry of this kind starts from. */
  defaults: EntryDraft
  /** Whether a draft carries the fields this kind requires to be saved. */
  isValid: (draft: EntryDraft) => boolean
  /** The one secret worth a shortcut — what ⏎ and ⌘⏎ copy. */
  primarySecret: (entry: Entry) => string
  /**
   * The values the list's other copy chords ask for — ⌘B the login name, ⌘U
   * the address, ⌘T the TOTP seed the code is cut from. Left off by kinds that
   * have no such field, so a chord on them does nothing.
   */
  username?: (entry: Entry) => string
  url?: (entry: Entry) => string
  otpSecret?: (entry: Entry) => string
  /** Untranslated label for the detail header's primary button. */
  primaryActionLabel: TKey
  /** The row's secondary line, from non-secret metadata only. */
  listSubtitle: (entry: EntryMeta) => string
  /**
   * A second eyebrow segment, cut from the decrypted entry — what kind of
   * document an identity is. Translated by the kind, since only it knows how to
   * phrase its own; `null` when the entry has nothing to add. Offered only
   * once the secrets are in hand, so it can name what is inside the payload.
   */
  eyebrow?: (entry: Entry) => { text: string; testid: string } | null
  /**
   * A second question the add picker asks before the form opens — which
   * document an identity is. Answered once, as a value seeded into `field` on
   * the new draft, so the form arrives already cut to the answer and never
   * changes shape under the user's hands. Left off by a kind with one form.
   */
  choices?: {
    /** The draft field the answer is written to. */
    field: string
    /** The question, as the picker's title. Untranslated. */
    title: TKey
    options: { value: string; label: TKey; Glyph: Glyph }[]
  }
  ListRow: ComponentType<ContentProps>
  /**
   * The kind's field set — the read view AND the editor. It takes no props:
   * the draft, the writer and the save-attempt flag all arrive through the
   * `FieldsProvider` its rows read (see components/elements/fields).
   */
  Fields: ComponentType
}
