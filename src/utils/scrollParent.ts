/**
 * The nearest ancestor that scrolls its content vertically, or null when only
 * the window does. What anything hung off a control has to stay inside: past
 * a scroller's edge it is cut off for good, however much room the window has.
 */
export const scrollParent = (el: Element): HTMLElement | null => {
  for (let node = el.parentElement; node; node = node.parentElement) {
    const overflow = getComputedStyle(node).overflowY
    if (overflow === 'auto' || overflow === 'scroll') return node
  }
  return null
}
