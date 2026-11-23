import { useTranslation } from 'react-i18next'
import type { Section as Key } from '@/store'
import { cx } from '@/utils/cx'
import Section from '../../Sidebar/Settings/Section'
import { descriptionOf, titleOf } from '../../Sidebar/Settings/sections'
import { useSubpage } from '../../Sidebar/Settings/sectionNav'
import SubpageBody from '../../Sidebar/Settings/SubpageBody'
import { subpageDescriptionOf, subpageTitleOf } from '../../Sidebar/Settings/subpages'
import BackButton from '../BackButton'
import NavBar from '../NavBar'
import Stack from '../Stack'
import { PUSHED_SCREEN, SCREEN_SCROLLER, TAB_BAR_CLEARANCE } from '../chrome'
import { useSwipeBack } from '../useSwipeBack'
import PaneHeading from './PaneHeading'

/**
 * One settings section, pushed from the root — and, pushed from it in turn,
 * any sub-page of it, and any page opened from that one.
 *
 * One level deep rather than modal: the tab bar stays up, so the scroller
 * reserves its clearance, and the way back is the shared `NavBar` carrying the
 * previous screen's name, exactly as the entry screen's row does — and, as
 * there, an edge swipe is that same control by gesture, held by the same lock.
 * The section stays under its sub-page (`Stack`), as the root stays under it,
 * and each sub-page under the one opened from it.
 */
export default function Pane({
  section,
  locked,
  onBack
}: {
  section: Key
  /** The section may not be left (see `settingsLocked`); Back is shown inert. */
  locked: boolean
  onBack: () => void
}) {
  const { t } = useTranslation()
  const { pages } = useSubpage()
  const swipe = useSwipeBack(onBack, locked)

  return (
    <Stack
      under={
        <div {...swipe.screen} data-testid="settings-pane" className={PUSHED_SCREEN}>
          <NavBar
            leading={
              <BackButton
                testid="settings-back"
                label={t('Settings')}
                disabled={locked}
                onClick={onBack}
              />
            }
          />

          <div className={cx(SCREEN_SCROLLER, 'px-4 pt-1', TAB_BAR_CLEARANCE)}>
            <PaneHeading
              title={titleOf(section)}
              description={descriptionOf(section)}
              className="mb-5"
            />
            <Section section={section} />
          </div>
          <div {...swipe.edge} />
        </div>
      }
      over={pages.length > 0 && <SubpagePane section={section} depth={0} locked={locked} />}
    />
  )
}

// The sub-page at `depth`, under whatever was opened from it. Its Back is
// named after the screen it leads to: the section, or the sub-page below.
//
// The heading stays put above the body: the frame scrolls itself and keeps the
// tab bar's clearance, so the column has no scroller of its own. `relative`:
// what the frame's bottom action is pinned to.
function SubpagePane({ section, depth, locked }: { section: Key; depth: number; locked: boolean }) {
  const { pages, close } = useSubpage()
  const swipe = useSwipeBack(close, locked)
  const subpage = pages[depth]
  const top = depth === pages.length - 1

  return (
    <Stack
      under={
        <div
          {...swipe.screen}
          // Only the page on top answers to the one test id.
          data-testid={top ? 'settings-subpage' : undefined}
          className={cx('relative', PUSHED_SCREEN)}
        >
          <NavBar
            leading={
              <BackButton
                testid={top ? 'settings-subpage-back' : undefined}
                label={depth > 0 ? subpageTitleOf(pages[depth - 1]) : titleOf(section)}
                disabled={locked}
                onClick={close}
              />
            }
          />
          <PaneHeading
            title={subpageTitleOf(subpage)}
            description={subpageDescriptionOf(subpage)}
            className="flex-none px-4 pt-1 pb-4"
          />
          <div className="flex min-h-0 flex-1 flex-col">
            <SubpageBody subpage={subpage} />
          </div>
          <div {...swipe.edge} />
        </div>
      }
      over={!top && <SubpagePane section={section} depth={depth + 1} locked={locked} />}
    />
  )
}
