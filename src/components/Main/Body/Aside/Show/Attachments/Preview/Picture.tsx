import { useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { isMobile } from '@/lib/platform'
import { cx } from '@/utils/cx'

interface Props {
  url: string
  name: string
  /** The engine could not decode the picture at `url`. */
  onError: () => void
}

/**
 * The picture on the stage, fitted to it — and on the desktop, a press away
 * from its actual size when it has more pixels than the stage can show, as
 * Preview and Quick Look offer: the pointer says which way (zoom in, zoom
 * out), the stage scrolls the picture at 1:1, and a second press fits it
 * again. A picture that already fits has nothing to zoom to, and the press
 * does nothing. A phone fits its pictures and leaves it there — pinching a
 * lightbox is its own piece of work, not this one.
 *
 * Whether the picture is larger than the stage is read when it loads and
 * again at the press, so a window resized since the load still gets the
 * right answer.
 */
export default function Picture({ url, name, onError }: Props) {
  const { t } = useTranslation()
  const image = useRef<HTMLImageElement | null>(null)
  const [actual, setActual] = useState(false)
  const [larger, setLarger] = useState(false)

  const overflows = () => {
    const el = image.current
    return !!el && (el.naturalWidth > el.clientWidth || el.naturalHeight > el.clientHeight)
  }

  const toggle = () => {
    if (isMobile) return
    if (actual) return setActual(false)
    if (overflows()) setActual(true)
  }

  const zoomable = !isMobile && (actual || larger)

  return (
    <img
      ref={image}
      src={url}
      alt={name}
      title={zoomable ? (actual ? t('Fit to window') : t('Actual size')) : undefined}
      data-testid="preview-image"
      data-zoom={actual ? 'actual' : 'fit'}
      onLoad={() => setLarger(overflows())}
      onError={onError}
      onClick={toggle}
      className={cx(
        // `m-auto` rather than the stage centring it: a flex item centred by
        // its parent is clipped at the start once it is larger than the
        // parent, and the picture at its actual size is meant to be.
        'm-auto md:rounded-md md:shadow-float',
        actual ? 'max-h-none max-w-none' : 'max-h-full max-w-full object-contain',
        zoomable && (actual ? 'cursor-zoom-out' : 'cursor-zoom-in')
      )}
    />
  )
}
