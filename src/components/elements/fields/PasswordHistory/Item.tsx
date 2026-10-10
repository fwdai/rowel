import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { PasswordHistoryItem } from '@/api/types'
import { useDates } from '@/hooks/useDates'
import { cx } from '@/utils/cx'
import { EyeGlyph, EyeOffGlyph } from '../../../Main/icons'
import CopyButton from '../../CopyButton'
import IconButton from '../../IconButton'
import { MASK_DOTS, META, ROW_COPY, ROW_HAIRLINE, VALUE } from '../../tokens'
import CopyValue from '../CopyValue'

// A finger's target where a finger is one of the pointers, as the field rail's.
const TOUCH = 'any-pointer-coarse:h-11 any-pointer-coarse:w-11'

// One previous password: masked until its eye is pressed, copyable either way
// (the value itself is the copy affordance, as on every read row — and on the
// phone the only one, see `ROW_COPY`), and when it was replaced.
export default function Item({ item, index }: { item: PasswordHistoryItem; index: number }) {
  const { t } = useTranslation()
  const { relativeLong } = useDates()
  const [show, setShow] = useState(false)
  const when = relativeLong(item.replacedAt)

  return (
    <div className={cx('flex items-center gap-1 py-2 pr-2 pl-3.5', ROW_HAIRLINE)}>
      <div className="min-w-0 flex-1">
        <CopyValue
          value={item.password}
          label={`${t('Previous password')} · ${t('Copy')}`}
          testid={`password-history-value-${index}`}
          className={cx(VALUE, 'w-full', show ? 'text-text tracking-secret' : 'text-text2')}
        >
          {show ? item.password : MASK_DOTS}
        </CopyValue>
        {when && <div className={META}>{t('Replaced {{when}}', { when })}</div>}
      </div>
      <IconButton
        title={show ? t('Hide') : t('Reveal')}
        active={show}
        className={TOUCH}
        testid={`password-history-reveal-${index}`}
        onClick={() => setShow(!show)}
      >
        {show ? <EyeOffGlyph /> : <EyeGlyph />}
      </IconButton>
      <span className={ROW_COPY}>
        <CopyButton
          value={item.password}
          title={t('Copy')}
          className={TOUCH}
          testid={`password-history-copy-${index}`}
        />
      </span>
    </div>
  )
}
