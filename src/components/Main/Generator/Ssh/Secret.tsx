import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import CopyButton from '@/components/elements/CopyButton'
import IconButton from '@/components/elements/IconButton'
import { LABEL } from '@/components/elements/tokens'
import { wellClass } from '@/components/elements/formStyles'
import { EyeGlyph, EyeOffGlyph } from '../../icons'

// A fixed mask, so the block says nothing about the key it is hiding.
const DOTS = '•'.repeat(48)

// The generated private key: masked until asked for, and copyable either way.
export default function Secret({ value }: { value: string }) {
  const { t } = useTranslation()
  const [show, setShow] = useState(false)

  return (
    <div>
      <div className={LABEL}>{t('Private key')}</div>
      <div className="mt-1 flex items-start gap-1.5">
        <div
          data-testid="generator-ssh-private"
          className={`min-w-0 flex-1 ${wellClass} px-3 py-2 font-mono text-base leading-relaxed break-all ${
            show ? 'max-h-40 overflow-auto whitespace-pre-wrap text-text' : 'text-text2'
          }`}
        >
          {show ? value : DOTS}
        </div>
        <IconButton
          title={show ? t('Hide') : t('Reveal')}
          active={show}
          testid="generator-ssh-reveal"
          onClick={() => setShow(!show)}
        >
          {show ? <EyeOffGlyph /> : <EyeGlyph />}
        </IconButton>
        <CopyButton value={value} title={t('Copy')} stay />
      </div>
    </div>
  )
}
