import { useState, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import type { TKey } from '@/i18n'
import { cx } from '@/utils/cx'
import { EyeGlyph, EyeOffGlyph } from '@/components/Main/icons'
import CopyButton from './CopyButton'
import IconButton from './IconButton'
import Panel from './Panel'
import Seal from './Seal'
import { grow } from './fields/grow'
import { ROW_PAD } from './fields/Row'
import { verbatimInput } from './inputProps'
import { MASK_INPUT, META, ROW_LABEL, TOUCH_TARGET } from './tokens'

interface Props {
  /** What the block holds, untranslated: "Note", "Private key". */
  label: TKey
  /** The draft key: `reveal-<name>`, `unseal-<name>`, `entry-value-<name>`, the box's `name`. */
  name: string
  /** The whole secret — what the copy button hands over, and what is typed. */
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
  /** A kind-local head action: generate a key, save a file. */
  actions?: ReactNode
  /** A body-specific cap or overflow rule. */
  bodyClassName?: string
  className?: string
  /**
   * Editing. Given a writer, the body is the box, under the same head: the
   * eye hides what is typed and the copy stands down, since a draft is not a
   * thing to take.
   */
  onChange?: (value: string) => void
  placeholder?: string
  /** What the editor has to say about the value: "Required". */
  error?: string
}

/**
 * A body-sized secret, in both modes: a secure note, an SSH private key, an
 * env file. The one shape for all — a panel whose head names the body, says
 * what it weighs, and carries the controls the body itself cannot be, over the
 * body as a blurred seal until the eye, or the seal, is pressed. Editing, the
 * body is a page to type on, in the same inks and padding, so the block keeps
 * its place and its size between reading and writing.
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
  className,
  onChange,
  placeholder,
  error
}: Props) {
  const { t } = useTranslation()
  const editing = !!onChange
  // Typing into dots is guesswork, so the editor starts revealed.
  const [shown, setShown] = useState(editing)
  const body = cx(
    'whitespace-pre-wrap bg-field px-3.5 py-3 text-text max-md:px-4',
    mono ? 'break-all font-mono text-xs leading-[1.65]' : 'break-words text-base leading-relaxed max-md:text-md',
    bodyClassName
  )

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
        {!editing && <CopyButton value={value} title={t('Copy')} className={TOUCH_TARGET} />}
      </div>

      {editing ? (
        <textarea
          name={name}
          aria-label={t(label)}
          value={value}
          // Four lines deep before a word is typed — the seal's own height —
          // and growing with what is typed from there.
          rows={4}
          placeholder={placeholder}
          {...verbatimInput}
          ref={grow}
          style={shown ? undefined : MASK_INPUT}
          onChange={event => {
            grow(event.currentTarget)
            onChange(event.target.value)
          }}
          // Not a flex item that grows: a textarea's minimum as one is its
          // `rows`, and the panel would hold it to four lines however much
          // was typed. `grow` sets its height, and the panel follows.
          className={cx('block w-full resize-none overflow-hidden outline-none placeholder:text-text2', body)}
        />
      ) : (
        <div className={cx('relative flex-1', body)} data-testid={`entry-value-${name}`}>
          {head && <div className="text-text2">{head}</div>}
          {shown ? (
            lines.map((line, index) => <div key={index}>{line}</div>)
          ) : (
            <Seal onReveal={() => setShown(true)} testid={`unseal-${name}`} />
          )}
          {foot && <div className="text-text2">{foot}</div>}
        </div>
      )}
      {error && <div className="px-3.5 py-2 text-base text-bad">{error}</div>}
    </Panel>
  )
}
