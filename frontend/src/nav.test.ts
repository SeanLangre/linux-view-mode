import { describe, expect, it } from 'vitest'
import { clamp, move, spatial, START, type Box, type RowSpec } from './nav'

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

describe('spatial', () => {
  const box = (col: number, row: number, width = 100): Box => ({ left: col * 120, top: row * 60, width, height: 50 })

  it('moves along a grid and stays put at the edges', () => {
    const boxes = [box(0, 0), box(1, 0), box(0, 1), box(1, 1)]
    expect(spatial(boxes, 0, 'right')).toBe(1)
    expect(spatial(boxes, 0, 'down')).toBe(2)
    expect(spatial(boxes, 3, 'left')).toBe(2)
    expect(spatial(boxes, 3, 'up')).toBe(1)
    expect(spatial(boxes, 0, 'left')).toBe(0)
    expect(spatial(boxes, 0, 'up')).toBe(0)
    expect(spatial(boxes, 3, 'right')).toBe(3)
  })

  it('goes down a list of full-width rows and from a short row to the list below', () => {
    const list = [box(0, 0, 300), box(0, 1, 300), box(0, 2, 300)]
    expect(spatial(list, 0, 'down')).toBe(1)
    expect(spatial(list, 2, 'up')).toBe(1)
    const mixed = [box(0, 0), box(1, 0), box(2, 0), box(0, 1, 360)]
    expect(spatial(mixed, 2, 'down')).toBe(3)
    expect(spatial(mixed, 3, 'up')).toBe(1) // the one nearest the middle of the wide row
  })

  it('prefers the button lined up over a nearer one off to the side', () => {
    const boxes = [box(1, 0), box(0, 1), box(1, 3)]
    expect(spatial(boxes, 0, 'down')).toBe(2)
  })

  it('handles empty and unknown positions', () => {
    expect(spatial([], 0, 'down')).toBe(-1)
    expect(spatial([box(0, 0)], -1, 'down')).toBe(0)
  })
})
