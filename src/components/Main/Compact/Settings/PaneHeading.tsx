/**
 * A pushed settings screen's name and what it is for: a section's pane and a
 * sub-page's alike, so the two read as one kind of screen.
 *
 * It names the screen the way a root does — there is no room for a centred
 * nav title beside a back control that already carries one — but a root's
 * title (`Heading`) shares a fixed row with its actions and has to stop at
 * one line. This one has the screen's width to itself, so a long title (a
 * sub-page's, in German) wraps rather than losing its end to an ellipsis.
 *
 * Where it sits is the caller's: in the section's scroller, or pinned above a
 * sub-page whose body scrolls itself.
 */
export default function PaneHeading({
  title,
  description,
  className
}: {
  title: string
  description: string
  className?: string
}) {
  return (
    <div className={className}>
      <h1 className="text-2xl font-semibold tracking-display break-words text-text">{title}</h1>
      <p className="mt-1 text-base text-text2">{description}</p>
    </div>
  )
}
