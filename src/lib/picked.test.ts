import { describe, it, expect } from 'vitest'
import { baseName, pickedPath } from './picked'

describe('baseName', () => {
  it('takes the last segment of either separator', () => {
    expect(baseName('/Users/me/code/api/.env')).toBe('.env')
    expect(baseName('C:\\code\\api\\.env.local')).toBe('.env.local')
    expect(baseName('.env')).toBe('.env')
  })
})

describe('pickedPath', () => {
  // A drop or an opened file is a path on disk, which ends in its name: what
  // the webview tells it by is exactly what it told it by before.
  it('names a path by its last component', () => {
    expect(pickedPath('/tmp/Rowel backup 2026-09-15.rowel')).toEqual({
      path: '/tmp/Rowel backup 2026-09-15.rowel',
      name: 'Rowel backup 2026-09-15.rowel'
    })
  })
})
