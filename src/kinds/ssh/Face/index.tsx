import Paper from '@/components/elements/Face'
import Cell from '@/components/elements/Face/Cell'
import FaceHeader from '@/components/elements/Face/Header'
import { useField } from '@/components/elements/fields'
import { randomart } from '@/utils/randomart'
import { keyLabel, parsePublicKey } from '../keyInfo'

/**
 * The fingerprint as the physical object — the read view's face of an SSH key.
 *
 * What ssh-keygen prints for a key, set on a plate: the randomart, so the key
 * is recognised at a glance, and the digest under it, which a click copies. The
 * chip names the algorithm and size, read off the public line. Reading only:
 * the fingerprint is never typed, so the editor keeps its plain row.
 */
export default function Face() {
  const fingerprint = useField('fingerprint').value.trim()
  const info = parsePublicKey(useField('publicKey').value)

  const art = randomart(fingerprint, keyLabel(info))
  const chip = keyLabel(info, ' · ')
  // "SHA256:…" — the hash names the caption; a fingerprint in another form
  // (an import's hex MD5) is shown whole with no caption to give it.
  const colon = fingerprint.indexOf(':')
  const hash = colon > 0 ? fingerprint.slice(0, colon) : ''
  const digest = colon > 0 ? fingerprint.slice(colon + 1) : fingerprint

  return (
    // The vault's shared face (see elements/Face) in its slate tone, which
    // follows the theme where a document's paper does not. Its own `@container`:
    // below 200px the plate draws in — tighter padding, the art a tier down — so
    // the 19-column picture still fits at the 160px floor its column can shrink
    // to (see `Read`). A flex column, because the grid stretches it to the rows'
    // height: the head stays at the top and the digest at the foot, and the
    // art takes the middle, whatever is left of it.
    <Paper
      tone="slate"
      data-testid="ssh-face"
      className="@container flex flex-col p-5 @max-[200px]:p-4"
    >
      <FaceHeader label="Fingerprint" chip={chip || undefined} />

      {/* The middle: the art centred in whatever height the rows leave it —
          and, on the phone's full-width plate, across it too, as a picture. */}
      <div className="flex flex-1 items-center py-4 max-md:justify-center">
        {art && (
          <div
            aria-hidden
            className="overflow-hidden whitespace-pre font-mono text-xs leading-[1.4] tracking-[0.02em] @max-[200px]:text-2xs"
          >
            {art.map((row, index) => (
              <div key={index}>{row}</div>
            ))}
          </div>
        )}
      </div>

      {/* The digest is a cell like any printed on a face: the hash as its
          caption, a press to copy, the mark in the plate's ink. */}
      <Cell
        name="fingerprint"
        label="Fingerprint"
        caption={hash}
        value={fingerprint}
        display={digest}
        ink="font-mono text-xs leading-[1.5]"
        wrap
      />
    </Paper>
  )
}
