import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { useApp } from '@/store'
import { providerName } from '@/api/provider'
import { syncProvider } from '@/api/sync'
import { TEXT_LINK } from '@/components/elements/tokens'
import { useSectionNav } from '../../sectionNav'

function Tile({ title, body }: { title: string; body: ReactNode }) {
  return (
    <div className="rounded-lg border border-line bg-field px-4 py-3">
      <div className="text-base font-medium text-text">{title}</div>
      <div className="mt-0.5 text-sm text-text2">{body}</div>
    </div>
  )
}

// No account to name, so the way to one. It lands on Your cloud storage, not
// the overview: "set one up" is a promise only that page's Connect keeps, and
// the overview says where things stand without controls of its own. The page
// sits under the section the link names, and its Back leads there.
function NoAccount() {
  const { t } = useTranslation()
  const go = useSectionNav()
  return (
    <>
      {t('No cloud account yet.')}{' '}
      <button
        type="button"
        data-testid="workspace-info-sync-link"
        onClick={() => go('sync', { key: 'sync-cloud' })}
        className={TEXT_LINK}
      >
        {t('Set one up in Sync & backup')}
      </button>
    </>
  )
}

// What a workspace is, in three facts, under the list of them. The third is
// where Lockboxes meets Sync & backup. Its title holds with or without an
// account (sync, wherever there is any, is per lockbox); its body names the
// open lockbox's account, the one a new lockbox would join, or with none the
// way to one.
export default function InfoTiles() {
  const { t } = useTranslation()
  const provider = useApp(state => syncProvider(state.sync))
  return (
    <div className="mt-3.5 grid grid-cols-1 gap-2.5 md:grid-cols-3">
      <Tile title={t('Separate database')} body={t('Each lockbox is its own encrypted file.')} />
      <Tile title={t('One unlock')} body={t('Your master password opens every lockbox that shares it.')} />
      <Tile
        title={t('Separate sync')}
        body={
          provider ? (
            t('Stored in the same {{provider}} account.', { provider: providerName(provider) })
          ) : (
            <NoAccount />
          )
        }
      />
    </div>
  )
}
