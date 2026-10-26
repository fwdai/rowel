import { useTranslation } from 'react-i18next'
import { cx } from '@/utils/cx'
import Frame from './Frame'
import { CARD } from './tokens'

interface Props {
  /** id of the title, for the frame's `aria-labelledby`; unique per sheet. */
  id: string
  title: string
  message?: string
  /** The verb, spelled out ("Remove attachment", "Approve device"), never "OK". */
  action: string
  /**
   * The verb's ink: the warning red of something that cannot be undone, or
   * the accent for a question that is weighty without being destructive — an
   * approval the user is asked to check a fingerprint for.
   */
  tone?: 'danger' | 'accent'
  onConfirm: () => void
  onClose: () => void
  testid: string
  actionTestid?: string
}

const ROW =
  'flex h-12 w-full cursor-pointer items-center justify-center text-md transition-colors hover:bg-hover active:bg-hover'

/**
 * "Are you sure?" the way a phone asks it: an action sheet from the bottom
 * edge with the question over two full-width rows — the destructive verb in
 * red, then a bold Cancel on a card of its own — and the scrim or a pull to
 * back out. The same content stands as a small card on the desktop, where
 * `Frame` is the centred dialog; the stacked rows read fine there too, and
 * Escape is the Cancel.
 *
 * `fit="content"`: a question and two rows need no more than the bottom edge.
 */
export default function ConfirmSheet({
  id,
  title,
  message,
  action,
  tone = 'danger',
  onConfirm,
  onClose,
  testid,
  actionTestid
}: Props) {
  const { t } = useTranslation()

  return (
    <Frame
      onClose={onClose}
      labelledBy={id}
      testid={testid}
      fit="content"
      hideClose
      className="w-full max-w-sm"
    >
      {/* The phone's sheet puts its grabber row above this; the desktop card
          has nothing there, so the question takes the top margin itself. */}
      <div className="px-5 pt-2 pb-5 md:pt-6">
        <div className="px-3 pb-4 text-center">
          <h1 id={id} className="text-base font-semibold text-text">
            {title}
          </h1>
          {message && <p className="mt-1 text-sm text-text2">{message}</p>}
        </div>
        <div className={CARD}>
          <button
            type="button"
            data-testid={actionTestid}
            onClick={onConfirm}
            className={cx(ROW, 'font-medium', tone === 'danger' ? 'text-bad' : 'text-accent')}
          >
            {action}
          </button>
        </div>
        <div className={cx(CARD, 'mt-2')}>
          <button
            type="button"
            data-testid="confirm-cancel"
            onClick={onClose}
            className={cx(ROW, 'font-semibold text-text')}
          >
            {t('Cancel')}
          </button>
        </div>
      </div>
    </Frame>
  )
}
