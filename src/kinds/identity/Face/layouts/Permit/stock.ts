import { ribbon, rosette } from '../../guilloche'
import type { Stock } from '..'

/** Green, a ribbon across it and a medallion run off its corner. */
export const STOCK: Stock = {
  wash: 'linear-gradient(118deg, rgba(70, 168, 126, 0.26), rgba(70, 168, 126, 0.09) 52%, rgba(216, 176, 98, 0.2))',
  line: '#2F8A66',
  engraving:
    rosette({ x: 418, y: 250, radius: 88, depth: 20, lobes: 13, strands: 6 }) +
    ribbon({ y: 118, depth: 26, length: 150, strands: 7, swell: 0.3 })
}
