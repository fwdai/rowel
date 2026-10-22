import { useState, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import type { TKey } from '@/i18n'
import { cx } from '@/utils/cx'
import { EyeGlyph, EyeOffGlyph } from '@/components/Main/icons'
import CopyButton from './CopyButton'
import IconButton from './IconButton'
import Panel from './Panel'
import Seal from './Seal'
import { ROW_PAD } from './fields/Row'
import { META, ROW_LABEL, TOUCH_TARGET } from './tokens'

interface Props {
  /** What the block holds, untranslated: "Note", "Private key". */
  label: TKey
  /** The draft key: `reveal-<name>`, `unseal-<name>` and `entry-value-<name>`. */
  name: string
  /** The whole secret — what the copy button hands over. */
  value: string
  /** What is known about the body without opening it: its format, its weight. */
  meta?: string
  /** Code rather than prose: the mono tier, broken anywhere it has to be. */
  mono?: boolean
  /** The sealed lines. A note is one line of pre-wrapped text; a key, many. */
  lines: string[]
  /** Lines kept legible around the seal — a key's BEGIN and END armor. */
  head?: string | null
  foot?: string | null
  /** A kind-local head action, such as saving an environment body as a file. */
  actions?: ReactNode
  /** A body-specific cap or overflow rule. */
  bodyClassName?: string
  className?: string
}

/**
 * A body-sized secret, read: a secure note, an SSH private key. The one shape
 * for both — a panel whose head names the body, says what it weighs, and
 * carries the two controls the body itself cannot be (a reveal and a copy),
 * over the body as a blurred seal until the eye, or the seal, is pressed.
 *
 * The seal is the only case in the vault where pressing a value does not copy
 * it: a block is read, not taken, so the copy lives in the head. Revealed, the
 * body reads in the primary ink like every other value; the armor around a key
 * stays in the second ink, since it is the envelope rather than the letter.
 */
export default function SealedBlock({
  label,
  name,
  value,
  meta,
  mono,
  lines,
  head,
  foot,
  actions,
  bodyClassName,
  className
}: Props) {
  const { t } = useTranslation()
  const [shown, setShown] = useState(false)

  return (
    <Panel className={cx('flex flex-col', className)}>
      <div className={cx('flex items-center gap-2.5 inset-shadow-hairline', ROW_PAD)}>
        {/* The rows' own caption face, so on the phone it is the grouped
            list's sentence-case caption like every row's above it. */}
        <span className={`flex-none whitespace-nowrap ${ROW_LABEL}`}>{t(label)}</span>
        {/* Takes the slack and truncates first: in a head too narrow for both,
            the name of the block outranks its weight. */}
        {meta && (
          <span className={`${META} min-w-0 flex-1 truncate text-right font-mono`}>{meta}</span>
        )}
        {actions}
        <IconButton
          title={shown ? t('Hide') : t('Reveal')}
          active={shown}
          testid={`reveal-${name}`}
          className={TOUCH_TARGET}
          onClick={() => setShown(!shown)}
        >
          {shown ? <EyeOffGlyph /> : <EyeGlyph />}
        </IconButton>
        <CopyButton value={value} title={t('Copy')} className={TOUCH_TARGET} />
      </div>

      <div
        className={cx(
          'relative flex-1 whitespace-pre-wrap bg-field px-3.5 py-3 text-text max-md:px-4',
          mono
            ? 'break-all font-mono text-xs leading-[1.65]'
            : 'break-words text-base leading-relaxed max-md:text-md',
          bodyClassName
        )}
        data-testid={`entry-value-${name}`}
      >
        {head && <div className="text-text2">{head}</div>}
        {shown ? (
          lines.map((line, index) => <div key={index}>{line}</div>)
        ) : (
          <Seal onReveal={() => setShown(true)} testid={`unseal-${name}`} />
        )}
        {foot && <div className="text-text2">{foot}</div>}
      </div>
    </Panel>
  )
}
