import { useTranslation } from 'react-i18next'
import { lockVault, type Section } from '@/store'
import { CARD } from '@/components/elements/tokens'
import { cx } from '@/utils/cx'
import { SECTIONS } from '../../Sidebar/Settings/sections'
import VaultFooter from '@/components/elements/VaultFooter'
import SyncIndicator from '../../Header/SyncIndicator'
import { LockGlyph } from '../../icons'
import { HEADER_ACTION, ROOT_HEADER, TAB_BAR_CLEARANCE } from '../chrome'
import Heading from '../Heading'
import Row from './Row'
import ArchiveRow from './ArchiveRow'

/**
 * The Settings root: the desktop's nav rail as a list of rows that push their
 * pane, the way an iOS settings screen reads. One flat list, in the rail's
 * order, without its group headings.
 *
 * It also holds what the vanished top bar used to. The sync chip rides in the
 * title row because it is a status pill; locking is a row of its own, in the
 * warning ink, because it ends the session and should look like it.
 */
export default function Root({ onSelect }: { onSelect: (section: Section) => void }) {
  const { t } = useTranslation()

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-screen pt-[env(safe-area-inset-top)]">
      <div className={`${ROOT_HEADER} px-4`}>
        <Heading title={t('Settings')} />
        {/* The chip deep-links here too, but through this screen's own
            navigation: the wide modal's section state is not what a pushed
            pane reads. */}
        <SyncIndicator className={HEADER_ACTION} onClick={() => onSelect('sync')} />
      </div>

      <div className={cx('min-h-0 flex-1 overflow-y-auto px-4 pt-5', TAB_BAR_CLEARANCE)}>
        <div className={CARD}>
          {SECTIONS.map(({ key, label, Glyph }) => (
            <Row
              key={key}
              testid={`settings-nav-${key}`}
              label={t(label)}
              glyph={<Glyph size={16} />}
              chevron
              onClick={() => onSelect(key)}
            />
          ))}
        </div>

        <div className={cx(CARD, 'mt-5')}>
          <ArchiveRow />
        </div>

        {/* The one place to lock on a phone: the top chrome that used to carry
            the button is gone, and the palette that carries the command is not
            reachable without a keyboard. */}
        <div className={cx(CARD, 'mt-5')}>
          <Row
            testid="lock-vault-button"
            label={t('Lock vault')}
            glyph={<LockGlyph size={16} />}
            ink="text-bad"
            onClick={() => void lockVault()}
          />
        </div>

        {/* The title row's chip already says where the vault lives. */}
        <VaultFooter sync={false} className="mt-4 px-1" />
      </div>
    </div>
  )
}
