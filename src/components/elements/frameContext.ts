import { createContext, useContext, type ComponentType } from 'react'
import Modal from './Modal'
import type { FrameProps } from './Frame'

// The card is the default, so a dialog rendered outside either shell — a test,
// an isolated screen — still comes up framed.
export const FrameContext = createContext<ComponentType<FrameProps>>(Modal)

/**
 * Whether the shell's frame draws its own close control. The phone frames
 * (a page's bar, a bottom sheet's top row) always do, so a dialog header inside
 * them leaves its own out rather than showing two ways to do the same thing.
 */
export const useFrameOwnsClose = () => useContext(FrameContext) !== Modal
