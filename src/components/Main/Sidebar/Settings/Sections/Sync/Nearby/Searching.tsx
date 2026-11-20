import { useTranslation } from 'react-i18next'
import SettingsRow from '@/components/elements/SettingsRow'
import Spinner from './Spinner'

/**
 * On, with no device found yet: what is happening, and the one thing that
 * helps. Shaped like the device rows it stands in for.
 */
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
