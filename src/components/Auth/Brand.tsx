import Mascot, { type MascotState } from '@/components/elements/Mascot'
import Wordmark from '@/components/elements/Wordmark'
import { cx } from '@/utils/cx'

interface Props {
  state: MascotState
  gaze: number
  /**
   * The mascot's measure. 96px where the masthead has a screen to itself; the
   * phone's lock screen steps it down while the keyboard is up, so the card
   * under it keeps the centre of what is left.
   */
  size?: number
  /** The gap to whatever follows — see below for the default. */
  className?: string
}

// The lock screen's masthead: the mascot with the wordmark under it. The gap
// between them gives the mascot room to stand, but stays well under the gap to
// whatever follows (mb-9), so the two group as one mark rather than as a
// figure and a caption. The mascot's cheer and shake happen above the name,
// which never moves.
export default function Brand({ state, gaze, size, className = 'mb-9' }: Props) {
  return (
    <div className={cx('flex flex-col items-center', className)}>
      <Mascot state={state} gaze={gaze} size={size} />
      <Wordmark className="mt-2.5" />
    </div>
  )
}
