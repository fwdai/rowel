/**
 * How often each distinct value occurs, busiest first and alphabetical among
 * equals: the order of every list that offers the vault's own values back
 * (its tags, the usernames it already holds).
 */
export const tally = (values: Iterable<string>): [value: string, count: number][] => {
  const totals = new Map<string, number>()
  for (const value of values) totals.set(value, (totals.get(value) ?? 0) + 1)

  return [...totals].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
}
