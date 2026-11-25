import { describe, it, expect } from 'vitest'
import { portraitOf } from './portrait'

const entry = (portrait?: unknown) =>
  ({ type: 'identity', title: 'Passport', portrait }) as Parameters<typeof portraitOf>[0]

describe('portraitOf', () => {
  it('hands back an inline JPEG as it is', () => {
    const uri = 'data:image/jpeg;base64,/9j/4AAQSkZJRg=='
    expect(portraitOf(entry(uri))).toBe(uri)
  })

  it('draws nothing an entry could have been made to carry instead', () => {
    expect(portraitOf(entry())).toBeUndefined()
    expect(portraitOf(entry(''))).toBeUndefined()
    expect(portraitOf(entry('https://example.test/me.jpg'))).toBeUndefined()
    expect(portraitOf(entry('data:image/svg+xml;base64,PHN2Zz4='))).toBeUndefined()
    expect(portraitOf(entry(['data:image/jpeg;base64,AA']))).toBeUndefined()
  })
})
