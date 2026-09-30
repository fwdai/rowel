import { useTranslation } from 'react-i18next'
import Button from '@/components/elements/Button'
import type { GeneratorDialog } from './useGeneratorDialog'

// Cancel and the one action that ends the dialog, in either body's bar.
export default function Actions({
  generator,
  onClose
}: {
  generator: GeneratorDialog
  onClose: () => void
}) {
  const { t } = useTranslation()
  const { ready, confirm, confirmLabel } = generator
  return (
    <>
      <Button variant="ghost" onClick={onClose}>
        {t('Cancel')}
      </Button>
      <Button testid="generator-use-button" kbd="⏎" onClick={confirm} disabled={!ready}>
        {t(confirmLabel)}
      </Button>
    </>
  )
}
