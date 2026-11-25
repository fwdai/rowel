import { rosette } from '../../guilloche'
import type { Stock } from '..'

/** Blue, with a medallion woven behind the holder's details. */
export const STOCK: Stock = {
  wash: 'linear-gradient(118deg, rgba(92, 138, 214, 0.24), rgba(92, 138, 214, 0.08) 52%, rgba(74, 184, 172, 0.2))',
  line: '#4A7AC0',
  engraving:
    rosette({ x: 322, y: 172, radius: 104, depth: 24, lobes: 14, strands: 7 }) +
    rosette({ x: 322, y: 172, radius: 52, depth: 14, lobes: 9, strands: 6 }) +
    rosette({ x: 36, y: 268, radius: 64, depth: 13, lobes: 12, strands: 5 })
}
