interface Props {
  size?: number
  className?: string
}

// A plain cloud, one colour, `currentColor`, so the caller inks it as
// `DropboxMark` is inked: `ProviderMark` gives it Apple's iCloud blue. Apple's
// own mark is a trademarked gradient asset and cannot be redrawn inline, so a
// generic cloud in the brand colour stands for it, as a glyph rather than a logo.
export default function ICloudMark({ size = 20, className }: Props) {
  return (
    <svg
      width={size}
      height={(size * 16) / 24}
      viewBox="0 4 24 16"
      aria-hidden
      className={className}
    >
      <path
        fill="currentColor"
        d="M19.35 10.04A7.49 7.49 0 0 0 12 4C9.11 4 6.6 5.64 5.35 8.04A5.994 5.994 0 0 0 0 14c0 3.31 2.69 6 6 6h13c2.76 0 5-2.24 5-5 0-2.64-2.05-4.78-4.65-4.96z"
      />
    </svg>
  )
}
