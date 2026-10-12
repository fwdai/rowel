import { useTranslation } from 'react-i18next'
import { useApp } from '@/store'
import { providerName } from '@/api/provider'
import { syncProvider } from '@/api/sync'

function Tile({ title, body }: { title: string; body: string }) {
  return (
    <div className="rounded-lg border border-line bg-field px-3.5 py-3">
      <div className="text-base font-medium text-text">{title}</div>
      <div className="mt-0.5 text-sm text-text2">{body}</div>
    </div>
  )
}

// What a workspace is, in three facts, under the list of them. The third names
// the open workspace's account, which is the one a new workspace would join.
export default function InfoTiles() {
  const { t } = useTranslation()
  const provider = useApp(state => syncProvider(state.sync))
  return (
    <div className="mt-3.5 grid grid-cols-1 gap-2.5 md:grid-cols-3">
      <Tile title={t('Separate database')} body={t('Each workspace is its own encrypted file.')} />
      <Tile title={t('One unlock')} body={t('Your master password opens every workspace that shares it.')} />
      <Tile
        title={t('Synced as its own vault')}
        body={
          provider
            ? t('Stored in the same {{provider}} account.', { provider: providerName(provider) })
            : t('Stored in the same cloud account.')
        }
      />
    </div>
  )
}
