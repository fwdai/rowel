/**
 * A stable hue for a string: the same seed always lands on the same colour,
 * and neighbouring seeds spread across the wheel rather than clustering. What
 * gives a workspace tile (`Monogram`, seeded by its id) and a tag chip
 * (`TagChip`, seeded by its name) a colour that is the thing's own, so it is
 * told apart at a glance before it is read — and the same thing is the same
 * colour wherever it is drawn.
 */
export const hueOf = (seed: string): number => {
  let hash = 5381
  for (let i = 0; i < seed.length; i++) hash = (hash * 33) ^ seed.charCodeAt(i)
  return Math.abs(hash) % 360
}
