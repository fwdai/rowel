import type { Theme } from '@/theme'

/**
 * What the Android activity puts on the window before the page loads
 * (`ShellBridge.kt`, as `__ROWEL_ANDROID__`): the three things about the
 * screen around the webview that only the activity can know or do. Absent on
 * every other platform, which is what makes everything here a no-op there.
 */
export interface AndroidShell {
  /**
   * How far the system bars and any display cutout reach in over each edge,
   * as JSON in device pixels: `{ top, right, bottom, left, bar }`. `bar` is
   * the part of `bottom` that is a navigation bar with buttons on it. `{}`
   * until the window has been measured.
   */
  insets(): string
  /** Draw the status and navigation bars' icons for a dark ground, or a light one. */
  setDark(dark: boolean): void
  /** Put the app in the background, as Home does. */
  leave(): void
}

/** Dispatched on the window by the activity whenever `insets()` has a new answer. */
export const INSETS_EVENT = 'rowel:insets'

// theme.css's inset properties, by the key the activity reports each under.
const PROPERTIES = {
  top: '--safe-top',
  right: '--safe-right',
  bottom: '--safe-bottom',
  left: '--safe-left',
  bar: '--bar-bottom'
} as const

const shell = (): AndroidShell | undefined => window.__ROWEL_ANDROID__

// Read rather than trusted, like the boot payload: anything that is not a
// number leaves its property at the stylesheet's default.
const measured = (bridge: AndroidShell): Record<string, unknown> => {
  try {
    const insets: unknown = JSON.parse(bridge.insets())
    return insets && typeof insets === 'object' ? (insets as Record<string, unknown>) : {}
  } catch {
    return {}
  }
}

const applyInsets = () => {
  const bridge = shell()
  if (!bridge) return
  const insets = measured(bridge)
  // The activity measures in device pixels; the page lays out in CSS ones.
  const ratio = window.devicePixelRatio || 1
  for (const [edge, property] of Object.entries(PROPERTIES)) {
    const px = insets[edge]
    if (typeof px === 'number' && Number.isFinite(px)) {
      document.documentElement.style.setProperty(property, `${px / ratio}px`)
    }
  }
}

/**
 * Keep theme.css's `--safe-*` and `--bar-bottom` at what the activity
 * measures, from now until the page goes: once here, so the first frame is
 * already clear of the bars, and again whenever the activity says they moved —
 * a rotation, a switch between gesture and button navigation, the keyboard
 * (which covers the bottom edge, so the bottom insets read zero while it is
 * up). A no-op off Android, where `env()` is right and the properties keep
 * their defaults.
 */
export const followSystemInsets = () => {
  if (!shell()) return
  applyInsets()
  window.addEventListener(INSETS_EVENT, applyInsets)
}

/**
 * Have the system bars' icons drawn for the ground the app paints under them.
 * The activity can only see the OS's light or dark, and the app's theme is its
 * own setting: left alone, a dark app on a light phone has dark icons on it.
 */
export const tintSystemBars = (theme: Theme) => shell()?.setDark(theme === 'dark')

/** Leave the app from its root, the way Back does on Android. */
export const leaveApp = () => shell()?.leave()
