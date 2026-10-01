import { useTranslation } from 'react-i18next'
import type { Section as Key } from '@/store'
import { cx } from '@/utils/cx'
import Section from '../../Sidebar/Settings/Section'
import { descriptionOf, titleOf } from '../../Sidebar/Settings/sections'
import { useSubpage } from '../../Sidebar/Settings/sectionNav'
import SubpageBody from '../../Sidebar/Settings/SubpageBody'
import {
  subpageCrumbOf,
  subpageDescriptionOf,
  subpageTitleOf,
  type Subpage
} from '../../Sidebar/Settings/subpages'
import BackButton from '../BackButton'
import NavBar from '../NavBar'
import { TAB_BAR_CLEARANCE } from '../chrome'
import { useSwipeBack } from '../useSwipeBack'

/**
 * One settings section, pushed from the root — or a sub-page of it, pushed
 * from the section.
 *
 * One level deep rather than modal: the tab bar stays up, so the scroller
 * reserves its clearance, and the way back is the shared `NavBar` carrying the
 * previous screen's name, exactly as the entry screen's row does — and, as
 * there, an edge swipe is that same control by gesture, held by the same lock.
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
  const { subpage } = useSubpage()
  const swipe = useSwipeBack(onBack, locked)

  if (subpage) return <SubpagePane subpage={subpage} locked={locked} />

  return (
    <div
      {...swipe}
      data-testid="settings-pane"
      className="flex min-h-0 flex-1 flex-col animate-sheet bg-screen"
    >
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

      <div className={cx('min-h-0 flex-1 overflow-y-auto px-4 pt-1', TAB_BAR_CLEARANCE)}>
        {/* The pane names itself the way a root does — there is no room for a
            centred nav title beside a back control that already carries one. */}
        <h1 className="truncate text-2xl font-semibold tracking-display text-text">
          {titleOf(section)}
        </h1>
        <p className="mt-1 mb-5 text-sm text-text2">{descriptionOf(section)}</p>
        <Section section={section} />
      </div>
    </div>
  )
}

// The heading stays put above the body: the frame scrolls itself and keeps the
// tab bar's clearance, so the column has no scroller of its own.
function SubpagePane({ subpage, locked }: { subpage: Subpage; locked: boolean }) {
  const { close } = useSubpage()
  const swipe = useSwipeBack(close, locked)

  return (
    <div
      {...swipe}
      data-testid="settings-subpage"
      className="flex min-h-0 flex-1 flex-col animate-sheet bg-screen"
    >
      <NavBar
        leading={
          <BackButton
            testid="settings-subpage-back"
            label={subpageCrumbOf(subpage)}
            disabled={locked}
            onClick={close}
          />
        }
      />
      <div className="flex-none px-4 pt-1 pb-4">
        <h1 className="truncate text-2xl font-semibold tracking-display text-text">
          {subpageTitleOf(subpage)}
        </h1>
        <p className="mt-1 text-sm text-text2">{subpageDescriptionOf(subpage)}</p>
      </div>
      <div className="flex min-h-0 flex-1 flex-col">
        <SubpageBody subpage={subpage} />
      </div>
    </div>
  )
}
