import type { TKey } from '@/i18n'
import { tintSystemBars } from '@/lib/android'

export type Theme = 'light' | 'dark'
export type ThemePreference = Theme | 'system'

/**
 * The accent choices, in the order the swatches offer them. Ink is the
 * default. Each id is a `[data-accent]` block in theme.css; the label is a
 * catalogue key. The palette is deliberately short and deliberately spread
 * around the wheel — one neutral, one cool, one blue, one green, one warm,
 * one red — so a choice is a choice and not a shade.
 */
export type Accent = 'ink' | 'petrol' | 'cobalt' | 'moss' | 'copper' | 'ruby'

export const ACCENTS: { id: Accent; label: TKey }[] = [
  { id: 'ink', label: 'Ink' },
  { id: 'petrol', label: 'Petrol' },
  { id: 'cobalt', label: 'Cobalt' },
  { id: 'moss', label: 'Moss' },
  { id: 'copper', label: 'Copper' },
  { id: 'ruby', label: 'Ruby' }
]

export const DEFAULT_ACCENT: Accent = 'ink'

export const isAccent = (value: unknown): value is Accent =>
  ACCENTS.some(accent => accent.id === value)

export const prefersDark = (): boolean => {
  try {
    return window.matchMedia?.('(prefers-color-scheme: dark)').matches ?? false
  } catch {
    return false
  }
}

export const resolveTheme = (next: ThemePreference): Theme =>
  next === 'system' ? (prefersDark() ? 'dark' : 'light') : next

export const applyTheme = (next: ThemePreference): void => {
  const theme = resolveTheme(next)
  document.documentElement.setAttribute('data-theme', theme)
  // Android draws its status and navigation bars' icons over this ground.
  tintSystemBars(theme)
}

/** The accent, like the theme, is a root attribute theme.css keys off. */
export const applyAccent = (next: Accent): void => {
  document.documentElement.setAttribute('data-accent', next)
}
