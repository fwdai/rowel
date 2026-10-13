import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'

// `shrinkPhoto` measures the engine once per module, so each test loads its own.
const load = async () => (await import('./photo')).shrinkPhoto

const CAMERA = new File([new Uint8Array(4_000_000)], 'IMG_0042.HEIC', { type: 'image/heic' })

const bitmap = (width: number, height: number) => ({ width, height, close: vi.fn() })

// The probe, then the photo: what this engine's `createImageBitmap` decodes.
const engine = (probe: { width: number; height: number }, photo: { width: number; height: number }) =>
  vi.fn((source: Blob) => Promise.resolve(source === CAMERA ? photo : probe))

const UPRIGHT = bitmap(1, 2)

let draw: ReturnType<typeof vi.fn>
let encoded: Blob | null

beforeEach(() => {
  vi.resetModules()
  draw = vi.fn()
  encoded = new Blob([new Uint8Array(400_000)], { type: 'image/jpeg' })
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({
    drawImage: draw
  } as unknown as CanvasRenderingContext2D)
  vi.spyOn(HTMLCanvasElement.prototype, 'toBlob').mockImplementation(callback =>
    callback(encoded)
  )
})

afterEach(() => {
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

describe('shrinkPhoto', () => {
  it('cuts a camera photo down to the longest side, as a JPEG under a .jpg name', async () => {
    const photo = bitmap(4032, 3024)
    vi.stubGlobal('createImageBitmap', engine(UPRIGHT, photo))
    const shrinkPhoto = await load()

    const shrunk = await shrinkPhoto(CAMERA)

    expect(shrunk).not.toBe(CAMERA)
    expect(shrunk.name).toBe('IMG_0042.jpg')
    expect(shrunk.type).toBe('image/jpeg')
    expect(shrunk.size).toBe(400_000)
    expect(draw).toHaveBeenCalledWith(photo, 0, 0, 2560, 1920)
    expect(photo.close).toHaveBeenCalled()
  })

  it('keeps the original where the engine has no createImageBitmap', async () => {
    vi.stubGlobal('createImageBitmap', undefined)
    const shrinkPhoto = await load()

    expect(await shrinkPhoto(CAMERA)).toBe(CAMERA)
  })

  it('keeps the original where the engine would leave the copy sideways', async () => {
    const decode = engine(bitmap(2, 1), bitmap(4032, 3024))
    vi.stubGlobal('createImageBitmap', decode)
    const shrinkPhoto = await load()

    expect(await shrinkPhoto(CAMERA)).toBe(CAMERA)
    expect(await shrinkPhoto(CAMERA)).toBe(CAMERA)
    // Measured once, and the photo itself never decoded.
    expect(decode).toHaveBeenCalledTimes(1)
  })

  it('keeps a photo that is already small enough', async () => {
    vi.stubGlobal('createImageBitmap', engine(UPRIGHT, bitmap(2560, 1440)))
    const shrinkPhoto = await load()

    expect(await shrinkPhoto(CAMERA)).toBe(CAMERA)
    expect(draw).not.toHaveBeenCalled()
  })

  it('keeps the original when the copy came out no smaller', async () => {
    encoded = new Blob([new Uint8Array(CAMERA.size)], { type: 'image/jpeg' })
    vi.stubGlobal('createImageBitmap', engine(UPRIGHT, bitmap(4032, 3024)))
    const shrinkPhoto = await load()

    expect(await shrinkPhoto(CAMERA)).toBe(CAMERA)
  })

  it('keeps the original when the engine cannot re-encode it', async () => {
    encoded = null
    vi.stubGlobal('createImageBitmap', engine(UPRIGHT, bitmap(4032, 3024)))
    const shrinkPhoto = await load()

    expect(await shrinkPhoto(CAMERA)).toBe(CAMERA)
  })

  it('keeps the original when the engine cannot decode it', async () => {
    vi.stubGlobal(
      'createImageBitmap',
      vi.fn((source: Blob) =>
        source === CAMERA ? Promise.reject(new Error('undecodable')) : Promise.resolve(UPRIGHT)
      )
    )
    const shrinkPhoto = await load()

    expect(await shrinkPhoto(CAMERA)).toBe(CAMERA)
  })
})
