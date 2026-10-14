import { fetchFavicon } from '@/api/tools'
import { imageLookup } from './imageLookup'

// Favicons for list rows, by host.
const favicons = imageLookup(fetchFavicon)

export const useFavicon = favicons.useImage

// Test seam and session end: the cache is module-level so it outlives
// per-test stores and would otherwise outlive the vault it was filled from.
export const resetFavicons = favicons.reset
