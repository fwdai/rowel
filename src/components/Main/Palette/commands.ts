import {
  usePrefs,
  lockVault,
  openAddPicker,
  openGenerator,
  openSettings,
  startEntry,
  toggleTheme,
  useCurrentEntry
} from '@/store'
import type { EntryMeta } from '@/api/types'
import { KINDS, addLabel, kindOf } from '@/kinds'
import { t, type TKey } from '@/i18n'
import { chord } from '@/lib/platform'
import { copyValue, type EntryValue } from '@/services/entries'
import {
  CopyGlyph,
  GeneratorGlyph,
  GearGlyph,
  LockGlyph,
  MoonGlyph,
  PlusGlyph,
  SunGlyph
} from '../icons'

type Glyph = typeof LockGlyph

export interface Command {
  id: string
  label: string
  // Displayed hint only; the binding itself lives in `useShortcuts` (or, for
  // the copy chords, the list's `copyKeys`).
  shortcut?: string
  glyph: Glyph
  run: () => void
}

// The selected entry's copy chords, listed so they can be found — and run from
// here, since the palette holds the keyboard while it is up. Only the values
// its kind carries; none for a tombstone, which has nothing to reveal.
const copyCommands = (entry: EntryMeta | null): Command[] => {
  if (!entry || entry.deletedAt) return []
  const kind = kindOf(entry.type)
  const offers: { value: EntryValue; label: TKey; key: string; has: boolean }[] = [
    { value: 'username', label: 'Copy username', key: 'B', has: !!kind.username },
    { value: 'secret', label: kind.primaryActionLabel, key: 'C', has: true },
    { value: 'otp', label: 'Copy one-time code', key: 'T', has: !!kind.otpSecret },
    { value: 'url', label: 'Copy URL', key: 'U', has: !!kind.url }
  ]
  return offers
    .filter(offer => offer.has)
    .map(offer => ({
      id: `copy-${offer.value}`,
      label: t(offer.label),
      shortcut: chord(offer.key),
      glyph: CopyGlyph,
      run: () => void copyValue(entry, offer.value)
    }))
}

// The palette's fixed command list. Order here is the order shown for an empty
// query, and the tie-break when several commands score the same. The per-kind
// commands come first and skip the picker: someone who already knows what they
// are saving should not be asked. The selected entry's copies trail the list,
// so opening the palette and pressing ⏎ still lands where it always has.
// Rebuilt on every render rather than memoized: a handful of objects is cheaper
// than the dependency bookkeeping, and `t` reads a module-level locale that a
// dependency array can't see.
export const useCommands = (): Command[] => {
  const theme = usePrefs(state => state.theme)
  const current = useCurrentEntry()

  return [
    ...KINDS.map(kind => ({
      id: `new-${kind.type}`,
      label: addLabel(kind.type),
      glyph: kind.Glyph,
      run: () => startEntry(kind.type)
    })),
    {
      id: 'add-secret',
      label: t('Add a secret'),
      shortcut: chord('N'),
      glyph: PlusGlyph,
      run: openAddPicker
    },
    {
      id: 'generate-password',
      label: t('Generate a password'),
      shortcut: chord('G'),
      glyph: GeneratorGlyph,
      // The standalone form: nothing to apply the value to, so it copies.
      run: () => openGenerator()
    },
    {
      id: 'lock-vault',
      label: t('Lock vault'),
      shortcut: chord('L'),
      glyph: LockGlyph,
      run: lockVault
    },
    {
      id: 'toggle-theme',
      label: t('Toggle theme'),
      glyph: theme === 'dark' ? SunGlyph : MoonGlyph,
      run: toggleTheme
    },
    { id: 'settings', label: t('Settings'), glyph: GearGlyph, run: openSettings },
    ...copyCommands(current)
  ]
}
