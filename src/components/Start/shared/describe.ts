/** The file name out of an OS path, for the backup the picker just handed back. */
export const fileNameOf = (path: string): string =>
  path.split(/[\\/]/).pop() || path
