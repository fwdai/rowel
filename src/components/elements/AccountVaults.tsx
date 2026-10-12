import { useTranslation } from 'react-i18next'
import type { RemotePack } from '@/api/setup'
import { useDates } from '@/hooks/useDates'
import { describeRemotePack } from '@/utils/remotePack'
import RadioList from './RadioList'
import FoundFileCard from './FoundFileCard'

interface Props {
  /** Every vault on offer, newest first. */
  files: RemotePack[]
  selectedId: string | null
  onSelect: (id: string) => void
  /**
   * Test id base: `<testid>-vault-<file id>` per option, `<testid>-found-file`
   * for the single card. The probe's picker keeps `drive`; a second list on
   * the same screen (the account's vaults not on this device) names its own.
   */
  testid?: string
}

/**
 * Which of the account's vaults this is about.
 *
 * More than one happens when separate installs have each synced their own
 * primary to the same account: two packs, two ids, and nothing but their dates
 * and sizes to tell them apart — so the choice is the user's, and it is made
 * before the password, which is per-vault. One vault is not a choice, so it is
 * shown rather than offered.
 *
 * Shared by the first run, Settings › Sync and Settings › Workspaces, which
 * pick a vault out of the same probe for the same reason.
 */
export default function AccountVaults({ files, selectedId, onSelect, testid = 'drive' }: Props) {
  const { t } = useTranslation()
  const dates = useDates()
  const file = files.find(candidate => candidate.id === selectedId) ?? null

  if (files.length > 1)
    return (
      <RadioList
        name={t('Vaults in this account')}
        testidPrefix={`${testid}-vault`}
        value={selectedId ?? ''}
        onChange={onSelect}
        options={files.map(vault => ({
          value: vault.id,
          label: t('Rowel vault'),
          meta: describeRemotePack(vault, dates)
        }))}
      />
    )

  return (
    file && (
      <FoundFileCard
        where="cloud"
        testid={`${testid}-found-file`}
        name={t('Rowel vault')}
        meta={describeRemotePack(file, dates)}
        encrypted
      />
    )
  )
}
