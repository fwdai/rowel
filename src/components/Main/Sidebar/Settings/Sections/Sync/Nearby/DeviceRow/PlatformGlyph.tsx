import type { NearbyPlatform } from '@/api/nearby'
import { DesktopGlyph, LaptopGlyph, PhoneGlyph } from '@/components/Main/icons'

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
