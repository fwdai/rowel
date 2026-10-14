import type { RemotePack } from '@/api/setup'
import type { Dates } from './time'
import { humanSize } from './size'

/**
 * The meta line under a found pack: how recently another device wrote it, and
 * how much there is. Both are metadata the provider already knows — nothing here
 * has been unsealed, and nothing here says what is inside. The date helpers are
 * the caller's (`useDates()`), so the line follows the pattern setting.
 *
 * Out here rather than beside the first run that needed it: the Settings flow
 * that restores a vault into a new workspace describes the same packs.
 */
export const describeRemotePack = (file: RemotePack, { relativeLong }: Dates): string =>
  [relativeLong(file.modifiedTime), humanSize(file.size)].filter(Boolean).join(' · ')
