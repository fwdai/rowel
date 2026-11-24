import type { PickedFile } from '@/api/tools'

/**
 * A file the user chose is two things: the address the backend reads it by
 * and the name it goes by (`PickedFile`). Everything the webview tells about
 * the file — its kind by its extension, the label it shows, the title it
 * proposes — comes from the name; the address is only handed back to read.
 * On a path the name is its last component, so the two only part on Android,
 * where a picked file's address is a document URI that ends in an id.
 */

/** The last component of an OS path. */
export const baseName = (path: string): string => path.split(/[\\/]/).pop() || path

/**
 * A file chosen by its path on disk — dropped on the window, or opened with
 * the app — which goes by its last component.
 */
export const pickedPath = (path: string): PickedFile => ({ path, name: baseName(path) })
