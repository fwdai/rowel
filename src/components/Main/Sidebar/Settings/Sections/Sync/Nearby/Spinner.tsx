/** Waiting on the network: the sync chip's accent ring, on its own. */
export default function Spinner() {
  return (
    <span
      aria-hidden
      className="block h-3.5 w-3.5 flex-none animate-spin rounded-full border-2 border-accent/25 border-l-accent"
    />
  )
}
