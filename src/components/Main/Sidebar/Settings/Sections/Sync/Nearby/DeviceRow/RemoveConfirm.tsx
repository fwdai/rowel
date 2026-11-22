import { useTranslation } from 'react-i18next'
import ConfirmSheet from '@/components/elements/ConfirmSheet'

interface Props {
  name: string
  onConfirm: () => void
  onClose: () => void
}

export default function RemoveConfirm({ name, onConfirm, onClose }: Props) {
  const { t } = useTranslation()

  return (
    <ConfirmSheet
      id="settings-nearby-remove-title"
      testid="settings-nearby-remove-sheet"
      actionTestid="settings-nearby-remove-confirm"
      title={t('Remove “{{name}}”?', { name })}
      message={t(
        'None of your devices will sync with it again. This doesn’t erase what’s already on it: if it was lost or stolen, also change your master password in Settings › Security.'
      )}
      action={t('Remove device')}
      onConfirm={onConfirm}
      onClose={onClose}
    />
  )
}
