import type { NearbyPlatform } from '@/api/nearby'
import { DesktopGlyph, LaptopGlyph, PhoneGlyph } from '@/components/Main/icons'

/** A device by what it is: a Mac, a PC, a phone. */
export default function PlatformGlyph({ platform }: { platform: NearbyPlatform }) {
  switch (platform) {
    case 'ios':
      return <PhoneGlyph />
    case 'macos':
      return <LaptopGlyph />
    case 'windows':
    case 'linux':
      return <DesktopGlyph />
  }
}
