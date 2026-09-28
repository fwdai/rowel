// The app's icon set: lucide, themed through `currentColor` so a `text-*`
// token colors every glyph in both themes.
//
// System rules (from the Keyring design system):
//   · stroke 1.75 — tracks the prototype's 1.7 hairline look (lucide's
//     default 2 reads too heavy at our sizes)
//   · three sizes — 14 inside 24–28px controls, 16 for rows and tiles, 20 for
//     the rail; call sites may override for hero surfaces
//
// Components keep the historical `*Glyph` names so this module stays the one
// place an icon choice lives.
import {
  Activity,
  Archive,
  ArchiveRestore,
  ArrowDownUp,
  AtSign,
  Braces,
  Calendar,
  Check,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  ClipboardList,
  Cloud,
  Copy,
  CreditCard,
  Dices,
  Download,
  HardDrive,
  ExternalLink,
  Eye,
  EyeOff,
  FileText,
  Fingerprint,
  Globe,
  IdCard,
  KeyRound,
  KeySquare,
  Layers,
  LayoutGrid,
  Link,
  Lock,
  Moon,
  MoreHorizontal,
  Palette,
  Pencil,
  Plus,
  Puzzle,
  RefreshCw,
  ScanLine,
  Search,
  Settings,
  Settings2,
  Share2,
  ShieldCheck,
  Star,
  Sun,
  Tags,
  Terminal,
  Trash2,
  TriangleAlert,
  Upload,
  User,
  X,
  createLucideIcon,
  type IconNode,
  type LucideIcon
} from 'lucide-react'

// Apple's Face ID mark, traced onto lucide's 24-unit grid: bracketed corners,
// two eye ticks, a nose hooking left, a short smile. The webview cannot draw
// the `faceid` SF Symbol itself, and lucide's ScanFace (dot eyes, no nose) does
// not read as Face ID next to the system's own prompt. Built with lucide's
// factory so it takes the same size/stroke/class handling as the rest of the set.
const faceIdNode: IconNode = [
  ['path', { d: 'M3 8.5V6a3 3 0 0 1 3-3h2.5' }],
  ['path', { d: 'M15.5 3H18a3 3 0 0 1 3 3v2.5' }],
  ['path', { d: 'M21 15.5V18a3 3 0 0 1-3 3h-2.5' }],
  ['path', { d: 'M8.5 21H6a3 3 0 0 1-3-3v-2.5' }],
  ['path', { d: 'M8.25 9.25v1.5' }],
  ['path', { d: 'M15.25 9.25v1.5' }],
  ['path', { d: 'M12.1 9.25v2.9a1 1 0 0 1-1 1h-.35' }],
  ['path', { d: 'M9.25 14.75a3.25 3.25 0 0 0 5.5 0' }]
]
const FaceId = createLucideIcon('face-id', faceIdNode)

// The sort glyph of the Rowel menus prototype: three rules stepping in from
// the left, flush on the right — a list settling into order. Lucide's own
// list-filter centres its rules and its sort arrows carry an arrow, so this
// one is drawn on the same grid instead.
const sortLinesNode: IconNode = [
  ['path', { d: 'M4 7h16' }],
  ['path', { d: 'M7 12h13' }],
  ['path', { d: 'M10 17h10' }]
]
const SortLines = createLucideIcon('sort-lines', sortLinesNode)

interface IconProps {
  size?: number
  className?: string
  // Solid rather than outline, for the glyphs that carry an on/off state (the
  // favorite star). Outline is lucide's default, so this is opt-in everywhere.
  filled?: boolean
  // A stroke in screen pixels whatever the size, for a glyph drawn off the
  // three sizes that has to hold a prototype's hairline (the lock chip's 10px
  // chevron at 1.5px). Left out, the stroke scales with the glyph.
  stroke?: number
}

const glyph =
  (Icon: LucideIcon, defaultSize: 14 | 16 | 20) =>
  ({ size = defaultSize, className, filled, stroke }: IconProps) => (
    <Icon
      size={size}
      strokeWidth={stroke ?? 1.75}
      absoluteStrokeWidth={stroke !== undefined}
      className={className}
      fill={filled ? 'currentColor' : 'none'}
    />
  )

// Control tier (24–28px hit areas)
export const SearchGlyph = glyph(Search, 14)
export const LockGlyph = glyph(Lock, 14)
export const SunGlyph = glyph(Sun, 14)
export const MoonGlyph = glyph(Moon, 14)
export const CopyGlyph = glyph(Copy, 14)
export const CheckGlyph = glyph(Check, 14)
export const MoreGlyph = glyph(MoreHorizontal, 14)
export const EyeGlyph = glyph(Eye, 14)
export const EyeOffGlyph = glyph(EyeOff, 14)
export const PencilGlyph = glyph(Pencil, 14)
// Really deletes (a field row, a purge). Archiving has its own pair below.
export const TrashGlyph = glyph(Trash2, 14)
export const ArchiveGlyph = glyph(Archive, 14)
export const ArchiveRestoreGlyph = glyph(ArchiveRestore, 14)
export const RefreshGlyph = glyph(RefreshCw, 14)
export const DownloadGlyph = glyph(Download, 14)
export const CloseGlyph = glyph(X, 14)
export const ChevronDownGlyph = glyph(ChevronDown, 14)
export const FingerprintGlyph = glyph(Fingerprint, 14)
// The other biometry: Face ID. Which of the two is shown comes from the device
// (`biometry_type`) through `lib/biometry` — never a per-call-site guess.
export const FaceIdGlyph = glyph(FaceId, 14)
export const SortGlyph = glyph(SortLines, 14)
export const ExternalGlyph = glyph(ExternalLink, 14)
export const AtGlyph = glyph(AtSign, 14)
export const UserGlyph = glyph(User, 14)
export const StarGlyph = glyph(Star, 14)
// The chrome's sync chip. Like the legacy indicator, the centre glyph says only
// *where* the vault lives -- disk or cloud -- and never changes with status; the
// state rides on the corner badge (CheckGlyph / AlertGlyph at 10) and the ring.
export const DiskGlyph = glyph(HardDrive, 14)
export const CloudGlyph = glyph(Cloud, 14)
export const AlertGlyph = glyph(TriangleAlert, 14)
// A passkey, on a login's detail rows and as the list-row mark.
export const KeyGlyph = glyph(KeyRound, 14)
// Handing one entry to someone else. The share mark rather than a link one: the
// link is how it travels, not what the action is, and `ExternalGlyph` already
// owns the chain-and-arrow look for "opens elsewhere".
export const ShareGlyph = glyph(Share2, 14)

// Row / tile / rail tier
export const PlusGlyph = glyph(Plus, 16)
export const GearGlyph = glyph(Settings, 16)
export const LoginGlyph = glyph(Globe, 16)
export const NoteGlyph = glyph(FileText, 16)
export const CardGlyph = glyph(CreditCard, 16)
export const IdentityGlyph = glyph(IdCard, 16)
// An SSH key. A shell prompt rather than a key: the key glyph is already the
// passkey's, and the login's is the globe — this has to read as neither.
export const SshGlyph = glyph(Terminal, 16)
// An env file. Braces rather than a document: the note already owns the page
// glyph and the SSH key the prompt, and `{ }` is what a file of KEY=VALUE
// lines is to the people who keep them — config, as code.
export const EnvGlyph = glyph(Braces, 16)
// An API key. A key on a tag rather than the round key the passkey mark wears:
// a token is a key someone issued, with the issuer's name on it.
export const ApiKeyGlyph = glyph(KeySquare, 16)
export const ScanGlyph = glyph(ScanLine, 16)
// The generator, as a row (the palette's command, the Add picker's action).
export const DicesGlyph = glyph(Dices, 16)
export const ShieldGlyph = glyph(ShieldCheck, 16)
export const GlobeGlyph = glyph(Globe, 16)
export const ActivityGlyph = glyph(Activity, 16)
export const LayersGlyph = glyph(Layers, 16)
// Settings › General: the app's own preferences, as sliders rather than the gear
// the rail already uses for Settings as a whole.
export const GeneralGlyph = glyph(Settings2, 16)
// Settings › Import & export: secrets travelling in both directions.
export const TransferGlyph = glyph(ArrowDownUp, 16)
// Settings › Browser extension: the puzzle piece browsers badge extensions with.
export const ExtensionGlyph = glyph(Puzzle, 16)
// The iOS disclosure mark: a row that pushes a screen rather than doing a thing.
export const ChevronRightGlyph = glyph(ChevronRight, 16)
// Settings row marks: the accent, the date pattern, the clipboard wipe.
export const PaletteGlyph = glyph(Palette, 16)
export const CalendarGlyph = glyph(Calendar, 16)
export const ClipboardGlyph = glyph(ClipboardList, 16)
// Links that are out there (Settings › Sharing). The row's own mark, not an
// action: `ShareGlyph` is what hands an entry over in the first place.
export const LinkGlyph = glyph(Link, 16)
// A file going into the app (the import drop target).
export const UploadGlyph = glyph(Upload, 16)

// Rail tier (20px, for the 40px+ rail hit areas and Settings nav headers)
export const PlusRailGlyph = glyph(Plus, 20)
export const GearRailGlyph = glyph(Settings, 20)
export const GridRailGlyph = glyph(LayoutGrid, 20)
export const StarRailGlyph = glyph(Star, 20)
export const TagsRailGlyph = glyph(Tags, 20)
export const ArchiveRailGlyph = glyph(Archive, 20)
export const LockRailGlyph = glyph(Lock, 20)
export const DicesRailGlyph = glyph(Dices, 20)
export const BackGlyph = glyph(ChevronLeft, 20)
