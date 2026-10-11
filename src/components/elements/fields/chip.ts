import { META_TYPE } from '../tokens'

// One chip — a scope, a short word in a hairline box at the meta size. A tag
// is not one of these: it has a shape of its own (`elements/TagChip`).
export const CHIP = `flex h-6 items-center rounded-sm border border-line2 px-2.5 ${META_TYPE} text-text2 transition-colors`
