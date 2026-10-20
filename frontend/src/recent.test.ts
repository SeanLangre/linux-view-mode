import { afterEach, expect, it, vi } from 'vitest'
import { addRecent, loadRecent } from './recent'

afterEach(() => vi.unstubAllGlobals())

it('keeps recent searches separate for Guest and each profile', () => {
  const storage = new Map<string, string>()
  vi.stubGlobal('localStorage', {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => storage.set(key, value),
  })
  addRecent('Music', 'alice')
  addRecent('Nature', 'bob')
  addRecent('Science', 'guest')
  expect(loadRecent('alice')).toEqual(['Music'])
  expect(loadRecent('bob')).toEqual(['Nature'])
  expect(loadRecent('guest')).toEqual(['Science'])
  expect(addRecent('music', 'alice')).toEqual(['music'])
})
