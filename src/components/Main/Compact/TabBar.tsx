import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import {
  useUi,
  setView,
  openGenerator,
  openSettings,
  closeSettings,
  closeGenerator,
  type View
} from '@/store'
import { cx } from '@/utils/cx'
import { DicesRailGlyph, GearRailGlyph, GridRailGlyph, StarRailGlyph } from '../icons'
import { TAB_BAR, TAB_BAR_FADE, TAB_BAR_NOTCH } from './chrome'
import Tab from './Tab'
import AddTab from './AddTab'

/**
 * The bar is five equal slots (`grid-cols-5`, and the lens's width below):
 * four tabs around an empty middle, where the notch is cut and the Add disc
 * rests. Which slot each destination holds is what the lens slides to.
 */
const SLOT: Record<'items' | 'favorites' | 'generator' | 'settings', number> = {
  items: 0,
  favorites: 1,
  generator: 3,
  settings: 4
}

/**
 * The phone's root navigation: a floating glass pill the content scrolls under.
 *
 * Four destinations — the two everyday lists, the generator and settings — with
 * the vault's one verb, Add, resting in a notch at the bar's centre: the
 * thumb's spot, rather than the far top corner it had in the list header. The
 * Archive is not a destination: it is where deleted things go, not somewhere
 * you spend a thumb, so it moved into Settings. The Tags view is not here
 * either — the bar has no slot left for it — so its tile and menu sit in the
 * list root's search field, and like the Archive it lights no tab.
 *
 * The selected tab is marked by one lens — the raised `bg-lens` key the
 * desktop rail's selected tile also wears, a pill here — that slides
 * from the old tab to the new one and settles with a little overshoot, the way
 * the system tab bar's does, rather than one wash blinking off and another on.
 *
 * Nothing here is new state: which tab is lit is read back off the store, and
 * a list tab closes whatever root is over it so the tap really navigates.
 */
export default function TabBar() {
  const { t } = useTranslation()
  const view = useUi(state => state.view)
  const settings = useUi(state => state.settings)
  // The standalone generator only. Opened from a password row it carries an
  // apply callback and is an overlay over the form, not a root.
  const generator = useUi(
    state => state.generator.open && !state.generator.apply && !state.generator.ssh
  )

  const list = (next: View) => () => {
    closeSettings()
    closeGenerator()
    setView(next)
  }
  const onList = (next: View) => !settings && !generator && view === next

  // Where the lens sits, or nowhere: the archive is a list view with no tab.
  const slot = settings
    ? SLOT.settings
    : generator
      ? SLOT.generator
      : view === 'items' || view === 'favorites'
        ? SLOT[view]
        : null
  // Where the lens was last lit. With no tab selected it fades out *there*;
  // falling back to the first slot would send it sliding home as it faded,
  // since the transform transitions along with the opacity. Adjusted during
  // render rather than in an effect so the lit position never lags a frame.
  const [parked, setParked] = useState(slot ?? SLOT.items)
  if (slot !== null && slot !== parked) setParked(slot)
  const at = slot ?? parked

  return (
    <>
      {/* Rows dissolve into the ground under the bar rather than sliding behind
          a hard edge. Purely decorative, so it never eats a tap. */}
      <div
        aria-hidden
        className={`pointer-events-none absolute inset-x-0 bottom-0 bg-linear-to-b from-transparent to-screen ${TAB_BAR_FADE}`}
      />
      {/* The shadow rides on this unmasked box: a mask clips everything its
          element paints to the border box, and a box-shadow lies wholly outside
          it, so on the pill itself the shadow was declared and never seen. */}
      <div className={`absolute z-10 rounded-full shadow-float ${TAB_BAR}`}>
        <nav
          data-testid="tab-bar"
          className={`relative grid h-full grid-cols-5 items-center rounded-full border border-line2 bg-glass px-1.5 backdrop-blur-xl ${TAB_BAR_NOTCH}`}
        >
          {/* The lens: one slot wide inside the bar's 6px paddings, moved by
              whole slots. Leaving for a view with no tab (the archive) fades it
              where it stands rather than sending it flying home. */}
          <span
            aria-hidden
            data-testid="tab-lens"
            style={{ transform: `translateX(${at * 100}%)` }}
            className={cx(
              'absolute inset-y-1 left-1.5 w-[calc((100%-12px)/5)] rounded-full bg-lens shadow-lens transition-[transform,opacity] duration-300 ease-spring',
              slot === null && 'opacity-0'
            )}
          />
          <Tab
            label={t('All Items')}
            testid="tab-items"
            selected={onList('items')}
            onClick={list('items')}
          >
            <GridRailGlyph />
          </Tab>
          <Tab
            label={t('Favorites')}
            testid="tab-favorites"
            selected={onList('favorites')}
            onClick={list('favorites')}
          >
            <StarRailGlyph />
          </Tab>
          {/* The notch's slot: nothing here, the disc rests over it. */}
          <div aria-hidden />
          {/* The two non-list roots are one screen slot between them, so each
              closes the other: tapping a tab always lands where it says. */}
          <Tab
            label={t('Generator')}
            testid="tab-generator"
            selected={generator}
            onClick={() => {
              closeSettings()
              openGenerator()
            }}
          >
            <DicesRailGlyph />
          </Tab>
          <Tab
            label={t('Settings')}
            testid="tab-settings"
            selected={settings}
            onClick={() => {
              closeGenerator()
              openSettings()
            }}
          >
            <GearRailGlyph />
          </Tab>
        </nav>
      </div>
      <AddTab />
    </>
  )
}
