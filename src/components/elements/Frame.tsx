import { useContext, type ComponentType, type ReactNode, type Ref } from 'react'
import { FrameContext } from './frameContext'

/**
 * What a dialog hands its frame, whichever frame it turns out to be.
 *
 * The two implementations read different halves of it — `Modal` the card's box
 * (`className`, `align`, `hideClose`), `Sheet` the phone's room budget (`fit`)
 * — and ignore the rest, which is what lets a dialog describe itself once
 * without knowing which shell it is in.
 */
export interface FrameProps {
  onClose: () => void
  /** id of the element that names the dialog, for `aria-labelledby`. */
  labelledBy?: string
  testid?: string
  /** Replaces the card's default box — its width, mostly. */
  className?: string
  /** Where the card sits on the scrim. */
  align?: 'top' | 'center'
  /**
   * How much room the body needs where room is scarce: a screen of its own, or
   * only as much as it takes. It describes the *content*, not the platform —
   * `Modal` always has the room and ignores it, and `Sheet` reads it to choose
   * between a full page and a bottom sheet.
   */
  fit?: 'screen' | 'content'
  /** For a body that carries its own close control. */
  hideClose?: boolean
  /** The frame element, for a caller that runs its own topmost-dialog check. */
  ref?: Ref<HTMLDivElement>
  children: ReactNode
}

/** Hands every `Frame` below it the implementation this shell wants. */
export const FrameProvider = ({
  value,
  children
}: {
  value: ComponentType<FrameProps>
  children: ReactNode
}) => <FrameContext value={value}>{children}</FrameContext>

/**
 * The frame the current shell puts its dialogs in: a centered card on desktop,
 * a full-screen sheet on a phone. Which one is decided once, in `Main`.
 */
export default function Frame(props: FrameProps) {
  const Implementation = useContext(FrameContext)
  // `react-hooks/static-components` cannot tell a component *chosen* from a
  // fixed pair (Modal / Sheet, both module-level) from one *defined* during
  // render. Choosing it here is the entire point of this file, and the shell
  // decides once in `Main` rather than per render, so nothing remounts.
  // eslint-disable-next-line react-hooks/static-components
  return <Implementation {...props} />
}
