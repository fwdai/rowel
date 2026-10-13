import { fetchEmblem } from '@/api/tools'
import { imageLookup } from './imageLookup'

// The coat of arms on a passport's cover, by ISO 3166-1 alpha-3 country.
const emblems = imageLookup(fetchEmblem)

export const useEmblem = emblems.useImage

// Emptied with the session, like the favicons: the countries it holds are the
// countries of the last vault's passports.
export const resetEmblems = emblems.reset
