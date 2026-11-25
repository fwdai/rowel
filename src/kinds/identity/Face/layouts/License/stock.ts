import { ribbon } from '../../guilloche'
import type { Stock } from '..'

/** Rose, the colour the card has had since it was paper, under two ribbons. */
export const STOCK: Stock = {
  wash: 'linear-gradient(118deg, rgba(226, 124, 160, 0.26), rgba(226, 124, 160, 0.09) 52%, rgba(122, 146, 226, 0.2))',
  line: '#C2527F',
  engraving:
    ribbon({ y: 84, depth: 20, length: 104, strands: 6, swell: 0.35 }) +
    ribbon({ y: 236, depth: 24, length: 138, strands: 7, swell: -0.3 })
}
