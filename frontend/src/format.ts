export function formatDuration(seconds: number | null): string {
  if (seconds === null) return 'LIVE'
  const s = Math.round(seconds)
  const h = Math.floor(s / 3600)
  const m = Math.floor((s % 3600) / 60)
  const sec = String(s % 60).padStart(2, '0')
  return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${sec}` : `${m}:${sec}`
}

export function formatViews(count: number | null): string | null {
  if (count === null) return null
  if (count >= 1e9) return `${trim(count / 1e9)}B views`
  if (count >= 1e6) return `${trim(count / 1e6)}M views`
  if (count >= 1e3) return `${trim(count / 1e3)}K views`
  return `${count} views`
}

/** "3 days ago" for a Unix timestamp in seconds; null if unknown. */
export function formatAge(timestamp: number | null | undefined, now = Date.now()): string | null {
  if (timestamp == null) return null
  const seconds = Math.max(0, now / 1000 - timestamp)
  const units: [string, number][] = [
    ['year', 31536000], ['month', 2592000], ['week', 604800], ['day', 86400], ['hour', 3600],
  ]
  for (const [name, size] of units) {
    const n = Math.floor(seconds / size)
    if (n >= 1) return `${n} ${name}${n > 1 ? 's' : ''} ago`
  }
  return 'Just now'
}

function trim(n: number): string {
  return n >= 10 ? Math.floor(n).toString() : n.toFixed(1).replace(/\.0$/, '')
}

/** Index of the lyric line being sung at `position` (lines are `[seconds, text]`, sorted), or -1 before the first. */
export function lyricLine(lines: [number, string][], position: number): number {
  let low = 0
  let high = lines.length
  while (low < high) {
    const mid = (low + high) >> 1
    if (lines[mid][0] <= position) low = mid + 1
    else high = mid
  }
  return low - 1
}
