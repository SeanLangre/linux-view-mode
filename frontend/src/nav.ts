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

export type Box = { left: number; top: number; width: number; height: number }

/**
 * Which of several on-screen boxes to focus when an arrow key is pressed: the nearest one in that
 * direction, preferring boxes that line up with the current one. Returns the index, or `current`
 * when nothing lies that way. Lets rows, grids and lists of different sizes share one screen.
 */
export function spatial(boxes: Box[], current: number, dir: Direction): number {
  const from = boxes[current]
  if (!from) return boxes.length > 0 ? 0 : -1
  const centre = (b: Box) => ({ x: b.left + b.width / 2, y: b.top + b.height / 2 })
  const a = centre(from)
  let best = current
  let bestScore = Infinity
  boxes.forEach((box, index) => {
    if (index === current) return
    const b = centre(box)
    const dx = b.x - a.x
    const dy = b.y - a.y
    const [along, across] = dir === 'left' || dir === 'right' ? [dx, dy] : [dy, dx]
    const sign = dir === 'right' || dir === 'down' ? 1 : -1
    // Must lie ahead (with a little tolerance for boxes of the same row or column).
    const ahead = along * sign
    if (ahead <= 1) return
    const aligned = Math.abs(across) < (dir === 'left' || dir === 'right' ? from.height : from.width) / 2
    // Boxes lined up with the current one beat the ones off to the side, however far away.
    const score = (aligned ? 0 : 1e6) + ahead + Math.abs(across) * 2
    if (score < bestScore) { bestScore = score; best = index }
  })
  return best
}
