import { describe, expect, it } from 'vitest'
import { formatAge, lyricLine } from './format'

describe('formatAge', () => {
  const now = 1_800_000_000_000
  it('names the largest unit', () => {
    expect(formatAge(now / 1000 - 3 * 86400, now)).toBe('3 days ago')
    expect(formatAge(now / 1000 - 86400, now)).toBe('1 day ago')
    expect(formatAge(now / 1000 - 400 * 86400, now)).toBe('1 year ago')
    expect(formatAge(now / 1000 - 60, now)).toBe('Just now')
  })
  it('is null when unknown', () => {
    expect(formatAge(null, now)).toBeNull()
    expect(formatAge(undefined, now)).toBeNull()
  })
})

describe('lyricLine', () => {
  const lines: [number, string][] = [[5, 'a'], [10, 'b'], [15, 'c']]
  it('finds the line being sung', () => {
    expect(lyricLine(lines, 1)).toBe(-1)
    expect(lyricLine(lines, 5)).toBe(0)
    expect(lyricLine(lines, 12)).toBe(1)
    expect(lyricLine(lines, 99)).toBe(2)
    expect(lyricLine([], 5)).toBe(-1)
  })
})
