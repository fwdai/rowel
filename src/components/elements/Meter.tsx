import { cx } from '@/utils/cx'
import { LEVEL_FILL } from './levels'

// Five-segment level readout, shared by the detail pane's zxcvbn strength bar
// and the generator's entropy bar. `level` is 0–4; `null` renders the empty rail.
// `stretch` shares its row's width between the segments instead of 22px each.
export default function Meter({ level, stretch }: { level: number | null; stretch?: boolean }) {
  return (
    <span className={cx('flex gap-[3px]', stretch ? 'flex-1' : 'flex-none')}>
      {[0, 1, 2, 3, 4].map(segment => (
        <span
          key={segment}
          className={cx(
            'h-[3px] rounded-full',
            stretch ? 'flex-1' : 'w-[22px]',
            level !== null && segment <= level ? LEVEL_FILL[level] : 'bg-line2'
          )}
        />
      ))}
    </span>
  )
}
