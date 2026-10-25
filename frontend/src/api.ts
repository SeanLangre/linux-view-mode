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
  /** Upload time, Unix seconds (approximate). */
  timestamp?: number | null
  resume_position?: number
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

function get<T>(path: string, refresh = false): Promise<T> {
  const cacheKey = `${profileScope}:${path}`
  const hit = cache.get(cacheKey)
  if (!refresh && hit && Date.now() - hit.at < CACHE_MS) return hit.promise as Promise<T>
  if (refresh) cache.delete(cacheKey)

  // `refresh=true` also makes the backend skip its own cache.
  const promise = fetch(refresh ? `${path}${path.includes('?') ? '&' : '?'}refresh=true` : path).then(async (res) => {
    const body = await res.json().catch(() => null)
    if (!res.ok) throw new Error(body?.error ?? `request failed (${res.status})`)
    return body as T
  })
  promise.catch(() => { if (cache.get(cacheKey)?.promise === promise) cache.delete(cacheKey) })
  cache.set(cacheKey, { at: Date.now(), promise })
  return promise
}

export const search = (query: string, refresh = false) =>
  get<Video[]>(`/api/search?q=${encodeURIComponent(query)}`, refresh)

/** Videos per page; a shorter page is the last one. Matches the backend. */
export const PAGE_SIZE = 24
/** The backend serves pages 0 to 9. */
export const MAX_PAGES = 10

export const channel = (id: string, page = 0, refresh = false) =>
  get<Channel>(`/api/channel/${encodeURIComponent(id)}?page=${page}`, refresh)

export type PlayerStatus = {
  playing: boolean
  id: string | null
  session?: number | null
  title: string | null
  position: number | null
  duration: number | null
  paused: boolean
  error: string | null
}

// Playback state must never use the search cache.
async function playback<T>(path: string, body?: object): Promise<T> {
  const res = await fetch(`/api/${path}`, body === undefined
    ? { cache: 'no-store' }
    : { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) })
  const data = await res.json().catch(() => null)
  if (!res.ok) throw new Error(data?.error ?? `request failed (${res.status})`)
  return data as T
}

export const play = (id: string, start?: number) => playback('play', { id, start })
export const stop = () => playback('stop', {})
export const playerStatus = () => playback<PlayerStatus>('status')

export type Profile = { id: string; name: string; color: string; subtitles: string | null }
export type ProfileList = { profiles: Profile[]; active_id: string }
export type LoginStatus = {
  state: 'guest' | 'unknown' | 'signing_in' | 'checking' | 'signed_in' | 'sign_in_again' | 'check_failed'
  message: string
}

export const profiles = () => playback<ProfileList>('profiles')
export const createProfile = (name: string, color: string) => playback<Profile>('profiles', { name, color })
export const selectProfile = (id: string) => playback<Profile>(`profiles/${encodeURIComponent(id)}/select`, {})
export async function setSubtitles(id: string, language: string | null) {
  const res = await fetch(`/api/profiles/${encodeURIComponent(id)}/subtitles`, {
    method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ language }),
  })
  const data = await res.json().catch(() => null)
  if (!res.ok) throw new Error(data?.error ?? `request failed (${res.status})`)
  return data as Profile
}
export const signin = async (id: string) => {
  const status = await playback<LoginStatus>(`profiles/${encodeURIComponent(id)}/signin`, {})
  for (const key of cache.keys()) { if (key.startsWith(`${id}:`)) cache.delete(key) }
  return status
}
export const loginStatus = (id: string, check = false) =>
  playback<LoginStatus>(`profiles/${encodeURIComponent(id)}/status${check ? '?check=true' : ''}`)
export type Pairing = { url: string; svg: string; code: string | null; confirmed: boolean }
export const pairing = () => playback<Pairing>('pairing')
export const startPairing = () => playback<Pairing>('pairing', {})
export const confirmPairing = (code: string) => playback<{ status: string }>('pairing/confirm', { code })
export const rejectPairing = () => playback<{ status: string }>('pairing/reject', {})
export const cancelPairing = () => fetch('/api/pairing', { method: 'DELETE' }).then(() => undefined, () => undefined)
export async function removeProfile(id: string) {
  const response = await fetch(`/api/profiles/${encodeURIComponent(id)}`, { method: 'DELETE' })
  const body = await response.json().catch(() => null)
  if (!response.ok) throw new Error(body?.error ?? `request failed (${response.status})`)
}

let profileScope = 'guest'
export function setProfileScope(id: string) {
  // Drop pending responses from the previous account as well as its browser cache.
  // The backend retains its own per-profile cache for instant reuse after switching back.
  cache.clear()
  profileScope = id
}

export type FeedKind = 'continue' | 'recommended' | 'subscriptions' | 'history' | 'watchlater' | 'liked'
export const feed = (kind: FeedKind, refresh = false, page = 0) =>
  playback<Video[]>(`feed/${kind}?page=${page}${refresh ? '&refresh=true' : ''}`)
export const queue = () => playback<Video[]>('queue')
export const enqueue = (id: string) => playback<Video[]>('queue', { id })
export const next = () => playback<{ playing: boolean }>('next', {})
export async function deleteQueue(id?: string) {
  const res = await fetch(`/api/queue${id ? `/${encodeURIComponent(id)}` : ''}`, { method: 'DELETE' })
  const body = await res.json().catch(() => null)
  if (!res.ok) throw new Error(body?.error ?? `request failed (${res.status})`)
}

export const exitToDesktop = () => playback<{ status: string }>('system/exit', {})

export const youtubeTv = () => playback<{ status: string }>('system/youtube-tv', {})
export const tvPowerAvailable = () => playback<{ available: boolean }>('system/tv')
export const tvPower = (on: boolean) => playback<{ status: string }>('system/tv', { on })

export type RemoteInfo = {
  enabled: boolean; devices: number
  url: string | null; svg: string | null; code: string | null; confirmed: boolean
}
export const remoteInfo = () => playback<RemoteInfo>('remote')
export async function setRemote(enabled: boolean) {
  const res = await fetch('/api/remote', {
    method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ enabled }),
  })
  const data = await res.json().catch(() => null)
  if (!res.ok) throw new Error(data?.error ?? `request failed (${res.status})`)
  return data as RemoteInfo
}
export const remotePair = () => playback<RemoteInfo>('remote/pair', {})
export const remoteConfirm = (code: string) => playback<{ status: string }>('remote/confirm', { code })
export const remoteReject = () => playback<{ status: string }>('remote/reject', {})
export async function forgetPhones() {
  const res = await fetch('/api/remote/devices', { method: 'DELETE' })
  if (!res.ok) throw new Error(`request failed (${res.status})`)
}
