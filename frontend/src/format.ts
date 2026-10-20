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

function trim(n: number): string {
  return n >= 10 ? Math.floor(n).toString() : n.toFixed(1).replace(/\.0$/, '')
}
