import { CHROME_BAR } from '@/components/elements/tokens'
import SyncIndicator from './SyncIndicator'

// Top chrome bar (38px): the sync chip alone, in the right corner. The bar is
// a status line, not a toolbar — one glance at its corner says whether the
// vault is up to date, and nothing else competes for the spot. The verbs live
// elsewhere: lock and settings at the foot of the rail, search in the list
// column it filters. Window controls are native on every OS, so the bar only
// has to keep clear of the macOS traffic lights, which the Overlay title-bar
// style pins ~8-20px from the window top and which nothing here can move
// (hence the left padding).
//
// `data-tauri-drag-region="deep"` makes the whole bar and its subtree a native
// window drag region: Tauri's injected handler starts a window drag on
// mousedown and, on macOS, zooms the window on double click (cancelled if the
// pointer moves, matching AppKit). "deep" is required because the bar's drag
// surface is mostly spacer children -- the bare attribute only reacts to clicks
// landing on the element itself.
//
// The handler already treats button/input/a/label/[role]/[tabindex] as
// non-drag, so the sync chip opts out for free.
export default function Header() {
  return (
    <header
      data-tauri-drag-region="deep"
      className={`relative z-10 flex ${CHROME_BAR} flex-none items-center border-b border-line bg-chrome pl-[78px] pr-3 backdrop-blur-[14px]`}
    >
      <div className="flex-1" />
      <SyncIndicator />
    </header>
  )
}
