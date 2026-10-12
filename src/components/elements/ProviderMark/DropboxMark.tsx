interface Props {
  size?: number
  className?: string
}

// The open-box glyph, five rhombi. One colour, `currentColor`, so the caller
// inks it; `ProviderMark` gives it the brand blue beside Drive's own colours.
export default function DropboxMark({ size = 20, className }: Props) {
  return (
    <svg
      width={size}
      height={(size * 20.4) / 24}
      viewBox="0 1.8 24 20.4"
      aria-hidden
      className={className}
    >
      <path
        fill="currentColor"
        d="M6 1.807 0 5.629l6 3.822 6.001-3.822zm12 0-6 3.822 6 3.822 6-3.822zM0 13.274l6 3.822 6.001-3.822L6 9.452zm18-3.822-6 3.822 6 3.822 6-3.822zM6 18.371l6.001 3.822 6-3.822-6-3.822z"
      />
    </svg>
  )
}
