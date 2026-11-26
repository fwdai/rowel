import type { TKey } from '@/i18n'

/**
 * The colours a card can be: the tints ID cards are usually printed in. A card
 * wears its document type's own unless the user picked another to match the
 * card in their hand — a residence permit is green here and blue in a wallet
 * often enough. The entry stores the key, never a colour value, so the palette
 * can be retuned without touching a vault.
 */
export const CARD_COLORS = ['rose', 'amber', 'green', 'teal', 'blue', 'violet', 'grey'] as const

export type CardColor = (typeof CARD_COLORS)[number]

interface Tint {
  label: TKey
  /** The hue the stock is washed in, as `r, g, b`, and the one it runs to. */
  from: string
  to: string
  /** The ink the ground and the card's mark are drawn in. */
  line: string
}

const TINTS: Record<CardColor, Tint> = {
  rose: { label: 'Rose', from: '226, 124, 160', to: '122, 146, 226', line: '#C2527F' },
  amber: { label: 'Amber', from: '226, 172, 86', to: '226, 132, 110', line: '#B07A1E' },
  green: { label: 'Green', from: '70, 168, 126', to: '216, 176, 98', line: '#2F8A66' },
  teal: { label: 'Teal', from: '64, 176, 180', to: '96, 150, 220', line: '#23838A' },
  blue: { label: 'Blue', from: '92, 138, 214', to: '74, 184, 172', line: '#4A7AC0' },
  violet: { label: 'Violet', from: '150, 124, 218', to: '214, 132, 190', line: '#7A5CC4' },
  grey: { label: 'Grey', from: '134, 142, 158', to: '176, 182, 194', line: '#6C7280' }
}

/**
 * A stored colour as a palette key, or undefined for anything that is not one
 * — the field is free text from a vault or an import, and '' means "the type's
 * own".
 */
export const cardColorOf = (value: unknown): CardColor | undefined =>
  (CARD_COLORS as readonly unknown[]).includes(value) ? (value as CardColor) : undefined

export const colorLabel = (color: CardColor): TKey => TINTS[color].label

export const lineOf = (color: CardColor): string => TINTS[color].line

/** The pale wash a card's stock is tinted with, under its printing. */
export const washOf = (color: CardColor): string => {
  const { from, to } = TINTS[color]
  return `linear-gradient(118deg, rgba(${from}, 0.26), rgba(${from}, 0.09) 52%, rgba(${to}, 0.2))`
}

/** The same two hues at full strength, on white: what a swatch shows. */
export const swatchOf = (color: CardColor): string => {
  const { from, to } = TINTS[color]
  return `linear-gradient(135deg, rgba(${from}, 0.9) 35%, rgba(${to}, 0.8) 130%), #fff`
}
