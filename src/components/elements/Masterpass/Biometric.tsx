import { useTranslation } from 'react-i18next'
import type { BiometryType } from '@/api/types'
import { biometryLabel } from '@/lib/biometry'
import { isMobile } from '@/lib/platform'
import BiometryGlyph from '../BiometryGlyph'
import IconButton from '../IconButton'
import Tooltip from '../Tooltip'

interface Props {
  biometry: BiometryType
  enter: boolean
  onClick?: () => void
}

// Biometrics are the card's own end segment: a taller divider than the reveal
// tier, and the glyph in the macOS Touch ID rose, sized to nearly fill the 28px
// button. Which glyph and name — Touch ID or Face ID — is `lib/biometry`'s
// call, from what the device reports.
export default function Biometric({ biometry, enter, onClick }: Props) {
  const { t } = useTranslation()
  const label = t(biometryLabel(biometry))

  const button = (
    <IconButton label={label} onClick={onClick}>
      {/* Child span so the rose survives IconButton's hover ink. */}
      <span className="text-touchid">
        <BiometryGlyph type={biometry} size={22} />
      </span>
    </IconButton>
  )

  return (
    <>
      <span aria-hidden className="my-auto h-7 w-px bg-line" />
      <div className="mx-1.5 flex items-center">
        {isMobile ? (
          button
        ) : (
          <Tooltip
            content={
              <span className="flex items-center gap-2">
                {label}
                {enter && <span className="opacity-60">⏎</span>}
              </span>
            }
          >
            {button}
          </Tooltip>
        )}
      </div>
    </>
  )
}
