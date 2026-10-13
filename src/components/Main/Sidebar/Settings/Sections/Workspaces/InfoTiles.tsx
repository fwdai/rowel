import { useTranslation } from 'react-i18next'

function Tile({ title, body }: { title: string; body: string }) {
  return (
    <div className="rounded-lg border border-line bg-field px-4 py-3">
      <div className="text-base font-medium text-text">{title}</div>
      <div className="mt-0.5 text-sm text-text2">{body}</div>
    </div>
  )
}

// What a workspace is, in three facts, under the list of them.
export default function InfoTiles() {
  const { t } = useTranslation()
  return (
    <div className="mt-3.5 grid grid-cols-1 gap-2.5 md:grid-cols-3">
      <Tile title={t('Separate database')} body={t('Each lockbox is its own encrypted file.')} />
      <Tile title={t('One unlock')} body={t('Your master password opens every lockbox that shares it.')} />
      <Tile
        title={t('Synced as its own lockbox')}
        body={t('Stored in the same Google Drive account.')}
      />
    </div>
  )
}
