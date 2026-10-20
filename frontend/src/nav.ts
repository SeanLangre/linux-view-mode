// 2D keyboard focus: the screen is a list of rows, each a list of items.
// Pure functions so they can be unit-tested without a browser.

export type RowSpec = {
  length: number
  /** Carousel rows remember their own column; grid rows keep the current column. */
  remember: boolean
}

export type Focus = {
  row: number
  col: number
  /** Last column used in each row, for rows with `remember`. */
  memory: Record<number, number>
}

export type Direction = 'up' | 'down' | 'left' | 'right'

export const START: Focus = { row: 0, col: 0, memory: {} }

/** Keeps focus on an existing item after rows change (e.g. data finished loading). */
export function clamp(focus: Focus, rows: RowSpec[]): Focus {
  let row = Math.min(Math.max(focus.row, 0), rows.length - 1)
  if (row < 0) return { ...focus, row: 0, col: 0 }

  if (rows[row].length === 0) {
    const target = nearestNonEmpty(rows, row)
    if (target === null) return { ...focus, row: 0, col: 0 }
    row = target
  }
  const col = Math.min(Math.max(focus.col, 0), rows[row].length - 1)
  return { ...focus, row, col }
}

export function move(focus: Focus, dir: Direction, rows: RowSpec[]): Focus {
  const cur = clamp(focus, rows)
  if (rows.length === 0 || rows[cur.row].length === 0) return cur

  if (dir === 'left' || dir === 'right') {
    const col = cur.col + (dir === 'right' ? 1 : -1)
    if (col < 0 || col >= rows[cur.row].length) return cur
    return { ...cur, col, memory: { ...cur.memory, [cur.row]: col } }
  }

  const step = dir === 'down' ? 1 : -1
  let row = cur.row + step
  while (row >= 0 && row < rows.length && rows[row].length === 0) row += step
  if (row < 0 || row >= rows.length) return cur

  const memory = { ...cur.memory, [cur.row]: cur.col }
  const wanted = rows[row].remember ? (memory[row] ?? 0) : cur.col
  return { row, col: Math.min(wanted, rows[row].length - 1), memory }
}

function nearestNonEmpty(rows: RowSpec[], from: number): number | null {
  for (let d = 1; d < rows.length; d++) {
    if (from - d >= 0 && rows[from - d].length > 0) return from - d
    if (from + d < rows.length && rows[from + d].length > 0) return from + d
  }
  return null
}
