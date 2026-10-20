import { describe, expect, it } from 'vitest'
import { clamp, move, START, type RowSpec } from './nav'

const carousel = (length: number): RowSpec => ({ length, remember: true })
const grid = (length: number): RowSpec => ({ length, remember: false })

describe('move', () => {
  it('moves left and right within a row and stops at the ends', () => {
    const rows = [carousel(3)]
    let f = move(START, 'right', rows)
    f = move(f, 'right', rows)
    expect(f.col).toBe(2)
    expect(move(f, 'right', rows).col).toBe(2)
    expect(move(START, 'left', rows).col).toBe(0)
  })

  it('stops at the first and last row', () => {
    const rows = [grid(1), grid(1)]
    expect(move(START, 'up', rows)).toMatchObject({ row: 0 })
    const last = move(START, 'down', rows)
    expect(move(last, 'down', rows)).toMatchObject({ row: 1 })
  })

  it('keeps the column in grid rows, clamped to the row length', () => {
    const rows = [grid(4), grid(4), grid(2)]
    let f = { ...START, col: 3 }
    f = move(f, 'down', rows)
    expect(f).toMatchObject({ row: 1, col: 3 })
    f = move(f, 'down', rows)
    expect(f).toMatchObject({ row: 2, col: 1 })
  })

  it('carousel rows remember their own column', () => {
    const rows = [carousel(10), carousel(10)]
    let f = move(START, 'right', rows) // row 0, col 1
    f = move(f, 'right', rows) // row 0, col 2
    f = move(f, 'down', rows)
    expect(f).toMatchObject({ row: 1, col: 0 })
    f = move(f, 'up', rows)
    expect(f).toMatchObject({ row: 0, col: 2 })
  })

  it('skips empty rows', () => {
    const rows = [grid(1), grid(0), grid(3)]
    expect(move(START, 'down', rows)).toMatchObject({ row: 2, col: 0 })
  })
})

describe('clamp', () => {
  it('pulls focus back onto existing items', () => {
    expect(clamp({ ...START, row: 5, col: 9 }, [grid(1), grid(3)])).toMatchObject({
      row: 1,
      col: 2,
    })
  })

  it('moves off a row that became empty', () => {
    expect(clamp({ ...START, row: 1 }, [grid(2), grid(0)])).toMatchObject({ row: 0 })
  })

  it('handles screens with nothing to focus', () => {
    expect(clamp({ ...START, row: 2, col: 2 }, [])).toMatchObject({ row: 0, col: 0 })
    expect(clamp(START, [grid(0)])).toMatchObject({ row: 0, col: 0 })
  })
})
