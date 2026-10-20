// Recent searches, kept in the kiosk browser's localStorage.
// Storage can be unavailable (private mode, cleared profile), so every access is guarded.

const PREFIX = 'couchtube.recentSearches'
const MAX = 8

export function loadRecent(profileId: string): string[] {
  try {
    const value = JSON.parse(localStorage.getItem(`${PREFIX}:${profileId}`) ?? '[]')
    return Array.isArray(value) ? value.filter((v) => typeof v === 'string') : []
  } catch {
    return []
  }
}

export function addRecent(query: string, profileId: string): string[] {
  const next = [query, ...loadRecent(profileId).filter((q) => q.toLowerCase() !== query.toLowerCase())]
    .slice(0, MAX)
  try {
    localStorage.setItem(`${PREFIX}:${profileId}`, JSON.stringify(next))
  } catch {
    // Not saved; the list still works for this session.
  }
  return next
}
