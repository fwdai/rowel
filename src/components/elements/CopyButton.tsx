import { useCopied } from '@/hooks/useCopied'
import { useTranslation } from 'react-i18next'
import IconButton from './IconButton'
import { CheckGlyph, CopyGlyph } from '../Main/icons'

export default function CopyButton({
  value,
  title,
  disabled,
  className,
  testid
}: {
  value: string
  title?: string
  // For a value that is not yet the one to copy (a generator mid-draw).
  disabled?: boolean
  className?: string
  testid?: string
}) {
  const { t } = useTranslation()
  const { copied, copy } = useCopied()
  return (
    <IconButton
      title={copied ? t('Copied') : title}
      disabled={disabled}
      className={className}
      testid={testid}
      onClick={() => copy(value)}
    >
      {copied ? <CheckGlyph className="text-good" /> : <CopyGlyph />}
    </IconButton>
  )
}
