// Client for the Rust backend's /api endpoints.

export type Video = {
  id: string
  title: string
  channel: string | null
  channel_id: string | null
  /** Seconds; null for live streams. */
  duration: number | null
  view_count: number | null
  thumb: string
}

export type Channel = {
  id: string
  name: string | null
  videos: Video[]
}

// yt-dlp takes 1–5 s per request, so results are reused for a while.
// This also makes going back to a screen instant.
const CACHE_MS = 10 * 60 * 1000
const cache = new Map<string, { at: number; promise: Promise<unknown> }>()

function get<T>(path: string): Promise<T> {
  const hit = cache.get(path)
  if (hit && Date.now() - hit.at < CACHE_MS) return hit.promise as Promise<T>

  const promise = fetch(path).then(async (res) => {
    const body = await res.json().catch(() => null)
    if (!res.ok) throw new Error(body?.error ?? `request failed (${res.status})`)
    return body as T
  })
  promise.catch(() => cache.delete(path))
  cache.set(path, { at: Date.now(), promise })
  return promise
}

export const search = (query: string) =>
  get<Video[]>(`/api/search?q=${encodeURIComponent(query)}`)

export const channel = (id: string) =>
  get<Channel>(`/api/channel/${encodeURIComponent(id)}`)
