import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from 'react'
import { useTranslation } from 'react-i18next'
import { isMobile } from '@/lib/platform'
import { cx } from '@/utils/cx'

interface Props {
  url: string
  name: string
  /** The engine could not decode the picture at `url`. */
  onError: () => void
}

/** A picture's size at one image pixel per device pixel, in CSS pixels. */
interface Size {
  width: number
  height: number
}

const actualSize = (el: HTMLImageElement): Size => {
  const scale = window.devicePixelRatio || 1
  return { width: el.naturalWidth / scale, height: el.naturalHeight / scale }
}

// Whether the engine can decode a picture off-screen and say so. jsdom
// cannot; there the picture is measured as it loads instead.
const canProbe =
  typeof HTMLImageElement !== 'undefined' && 'decode' in HTMLImageElement.prototype

/**
 * The picture on the stage: fitted to it, but never past its actual size —
 * a small picture stands at that size in the middle of a large window rather
 * than being blown up to fill it, as Preview and Quick Look show one. Actual
 * size is one image pixel per device pixel, so a Retina screenshot stands at
 * the size it was taken at and not twice it.
 *
 * The size is known before the picture is shown: the bytes are decoded
 * off-screen first (`decode()`), and the `<img>` is mounted with its bound
 * already on it. Measured on load instead, the first frame would be the
 * picture filling the stage and the next the picture at its size — a jump
 * on every step through the list. The decoded bitmap is the engine's cache,
 * so the picture then paints whole and at once.
 *
 * On the desktop a picture with more pixels than the stage can show is a
 * press away from that actual size: the pointer says which way (zoom in,
 * zoom out), the stage scrolls the picture at 1:1, and a second press fits
 * it again. A picture that already fits has nothing to zoom to, and the
 * press does nothing. A phone fits its pictures and leaves it there —
 * pinching a lightbox is its own piece of work, not this one.
 *
 * Whether the picture is larger than the stage is read once it is laid out
 * and again at the press, so a window resized since still gets the right
 * answer.
 */
export default function Picture({ url, name, onError }: Props) {
  const { t } = useTranslation()
  const image = useRef<HTMLImageElement | null>(null)
  const [actual, setActual] = useState(false)
  const [size, setSize] = useState<Size | null>(null)
  const [larger, setLarger] = useState(false)
  // The latest handler, for a decode that finishes after a re-render: the
  // probe is tied to the URL alone, not to the handler's identity.
  const fail = useRef(onError)
  useEffect(() => {
    fail.current = onError
  }, [onError])

  useEffect(() => {
    if (!canProbe) return
    let alive = true
    const probe = new Image()
    probe.src = url
    probe.decode().then(
      () => alive && setSize(actualSize(probe)),
      () => alive && fail.current()
    )
    return () => {
      alive = false
    }
  }, [url])

  // Shown smaller than it is: the fit had to shrink it.
  const shrunk = () => {
    const el = image.current
    if (!el) return false
    const full = actualSize(el)
    return el.clientWidth < full.width - 0.5 || el.clientHeight < full.height - 0.5
  }

  // Laid out with its bound on: now the stage's room is known.
  useLayoutEffect(() => {
    if (size) setLarger(shrunk())
  }, [size])

  // The engine that cannot probe measures the picture as it loads.
  const loaded = () => {
    if (!canProbe && image.current) setSize(actualSize(image.current))
  }

  const toggle = () => {
    if (isMobile) return
    if (actual) return setActual(false)
    if (shrunk()) setActual(true)
  }

  if (canProbe && !size) return null

  const zoomable = !isMobile && (actual || larger)

  // Fitted: no larger than the stage and no larger than itself. Actual: itself.
  const bounds: CSSProperties | undefined = size
    ? actual
      ? { width: size.width, height: size.height }
      : { maxWidth: `min(100%, ${size.width}px)`, maxHeight: `min(100%, ${size.height}px)` }
    : undefined

  return (
    <img
      ref={image}
      src={url}
      alt={name}
      title={zoomable ? (actual ? t('Fit to window') : t('Actual size')) : undefined}
      data-testid="preview-image"
      data-zoom={actual ? 'actual' : 'fit'}
      style={bounds}
      onLoad={loaded}
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
