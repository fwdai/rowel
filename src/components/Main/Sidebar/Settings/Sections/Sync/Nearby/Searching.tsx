import { useTranslation } from 'react-i18next'
import SettingsRow from '@/components/elements/SettingsRow'
import Spinner from './Spinner'

export default function Searching() {
  const { t } = useTranslation()

  return (
    <SettingsRow
      testid="settings-nearby-searching"
      icon={<Spinner />}
      label={t('Looking for your devices…')}
      description={t('Open {{appName}} on another device on this network.')}
    />
  )
}
