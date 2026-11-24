import type { ReactNode } from 'react'

interface Props {
  /** The way back or out, on the left edge. */
  leading?: ReactNode
  /**
   * The centred nav title. Omitted where `leading` already names the screen —
   * there is no room for two names on a 390px row.
   */
  title?: string
  /** The screen's own actions, on the right edge. */
  trailing?: ReactNode
}

/**
 * A pushed screen's header: 56px of controls over the safe area, leading /
 * title / trailing, the iOS way round.
 *
 * One file rather than three because the three pushed screens (the entry, the
 * form, a settings pane) only differ in what they put in the slots — the
 * geometry underneath was drifting apart every time one of them was touched.
 */
export default function NavBar({ leading, title, trailing }: Props) {
  return (
    <header className="flex flex-none items-center gap-1 px-2 pt-(--safe-top)">
      {leading}
      {title === undefined ? (
        // No title to centre, so the trailing cluster takes the slack itself.
        <div className="flex-1" />
      ) : (
        <h1 className="min-w-0 flex-1 truncate text-center text-md font-semibold tracking-display text-text">
          {title}
        </h1>
      )}
      {trailing && <div className="flex flex-none items-center gap-0.5">{trailing}</div>}
    </header>
  )
}
