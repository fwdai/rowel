import { useCopied } from '@/hooks/useCopied'
import { useTranslation } from 'react-i18next'
import IconButton from './IconButton'
import { CheckGlyph, CopyGlyph } from '../Main/icons'

export default function CopyButton({
  value,
  title,
  disabled,
  className,
  testid,
  stay
}: {
  value: string
  title?: string
  // For a value that is not yet the one to copy (a generator mid-draw).
  disabled?: boolean
  className?: string
  testid?: string
  // Keep Rowel up even with "Hide Rowel after copying" on (see `CopyOptions`).
  stay?: boolean
}) {
  const { t } = useTranslation()
  const { copied, copy } = useCopied()
  return (
    <IconButton
      title={copied ? t('Copied') : title}
      disabled={disabled}
      className={className}
      testid={testid}
      onClick={() => copy(value, { stay })}
    >
      {copied ? <CheckGlyph className="text-good" /> : <CopyGlyph />}
    </IconButton>
  )
}
