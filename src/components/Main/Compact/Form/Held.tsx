import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import type { EntryMeta, EntryType } from '@/api/types'
import { setNoEntry } from '@/store'
import NavBar from '../NavBar'
import NavButton from '../NavButton'
import { useHeading } from './useHeading'

interface Props {
  entry?: EntryMeta
  type: EntryType
  /** What to say in the frame, if anything — a reveal that failed, for one. */
  children?: ReactNode
}

/**
 * The form screen's frame, held until the secrets are in hand — an editor
 * seeded from a reveal that has not landed would discard whatever is typed
 * meanwhile.
 *
 * It carries a working Cancel rather than being a bare box: a reveal that never
 * arrives (an offline vault, a backend that refuses) would otherwise leave the
 * screen with no way off it. Cancel drops the whole entry state, which is where
 * this screen was pushed from.
 *
 * On the way in from the read screen this never renders — that screen already
 * revealed the entry and `Compact/Entry` shares the one reveal between the two
 * faces. Only an edit asked for before the first reveal landed gets here.
 */
export default function Held({ entry, type, children }: Props) {
  const { t } = useTranslation()
  const heading = useHeading(type, entry)

  return (
    <div className="flex min-h-0 flex-1 flex-col animate-rise bg-screen text-text">
      <NavBar
        title={heading}
        leading={
          <NavButton testid="cancel-entry-button" onClick={setNoEntry}>
            {t('Cancel')}
          </NavButton>
        }
      />
      <div className="flex min-h-0 flex-1 flex-col px-4">{children}</div>
    </div>
  )
}
