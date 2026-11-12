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
  /** Watched to the end by this profile. */
  completed?: boolean
  /** Why For You suggested it. */
  reason?: string
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

// When the backend answers from an old saved copy (it says so in `x-cache: stale`) it is already
// fetching a new one. Ask again a few seconds later and tell the open screen if the list changed.
const refreshListeners = new Set<(path: string) => void>()
export function onListRefreshed(listener: (path: string) => void) {
  refreshListeners.add(listener)
  return () => { refreshListeners.delete(listener) }
}
const REFRESH_AFTER_MS = 6000
const REFRESH_TRIES = 3

function refreshLater(path: string, cacheKey: string, scope: string, previous: string, attempt: number) {
  window.setTimeout(async () => {
    if (scope !== profileScope) return // another profile is showing now
    try {
      const res = await fetch(path, { cache: 'no-store' })
      if (!res.ok) return
      const body = await res.json()
      const text = JSON.stringify(body)
      if (text !== previous) {
        cache.set(cacheKey, { at: Date.now(), promise: Promise.resolve(body) })
        refreshListeners.forEach((listener) => listener(path))
      }
      if (res.headers.get('x-cache') === 'stale' && attempt < REFRESH_TRIES) refreshLater(path, cacheKey, scope, text, attempt + 1)
    } catch { /* the next visit asks again */ }
  }, REFRESH_AFTER_MS)
}

function get<T>(path: string, refresh = false): Promise<T> {
  const cacheKey = `${profileScope}:${path}`
  const scope = profileScope
  const hit = cache.get(cacheKey)
  if (!refresh && hit && Date.now() - hit.at < CACHE_MS) return hit.promise as Promise<T>
  if (refresh) cache.delete(cacheKey)

  // `refresh=true` also makes the backend skip its own cache.
  const promise = fetch(refresh ? `${path}${path.includes('?') ? '&' : '?'}refresh=true` : path).then(async (res) => {
    const body = await res.json().catch(() => null)
    if (!res.ok) throw new Error(body?.error ?? `request failed (${res.status})`)
    if (res.headers?.get('x-cache') === 'stale') refreshLater(path, cacheKey, scope, JSON.stringify(body), 1)
    return body as T
  })
  promise.catch(() => { if (cache.get(cacheKey)?.promise === promise) cache.delete(cacheKey) })
  cache.set(cacheKey, { at: Date.now(), promise })
  return promise
}

export const search = (query: string, refresh = false) =>
  get<Video[]>(`/api/search?q=${encodeURIComponent(query)}`, refresh)

/** Videos per page. Matches the backend, which drops Shorts, so a page can be shorter; only an empty page is the last one. */
export const PAGE_SIZE = 24
/** The backend serves pages 0 to 9. */
export const MAX_PAGES = 10

export const channel = (id: string, page = 0, refresh = false) =>
  get<Channel>(`/api/channel/${encodeURIComponent(id)}?page=${page}`, refresh)

export type PlayerStatus = {
  playing: boolean
  /** Player page this window must show (playback in the browser rather than mpv). */
  page?: string | null
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
  // Feeds come through here (never cached in the browser). If the backend showed an old saved copy,
  // ask again shortly and tell the open screen when the list changed.
  if (body === undefined && res.headers?.get('x-cache') === 'stale') {
    refreshLater(`/api/${path}`, `${profileScope}:/api/${path}`, profileScope, JSON.stringify(data), 1)
  }
  return data as T
}

export type BuildVersion = { version: string; commit: string }
let buildVersion: Promise<BuildVersion | null> | undefined
/** Which build is running; asked once. `null` if the backend does not answer. */
export const version = () => (buildVersion ??= playback<BuildVersion>('version').catch(() => null))

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

export type Mode = 'youtube' | 'twitch' | 'music'
export const getMode = () => playback<{ mode: Mode | null }>('mode').then((r) => r.mode)
export const setMode = (mode: Mode) =>
  fetch('/api/mode', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ mode }) })
    .then(async (res) => { if (!res.ok) throw new Error((await res.json().catch(() => null))?.error ?? `request failed (${res.status})`) })

/** Twitch profiles are their own list; they share nothing with the YouTube profiles above. */
export type TwitchProfile = { id: string; name: string; color: string }
export const twitchProfiles = () => playback<{ profiles: TwitchProfile[]; active_id: string }>('twitch/profiles')
export const createTwitchProfile = (name: string, color: string) => playback<TwitchProfile>('twitch/profiles', { name, color })
export const selectTwitchProfile = (id: string) => playback<TwitchProfile>(`twitch/profiles/${encodeURIComponent(id)}/select`, {})
export async function removeTwitchProfile(id: string) {
  const response = await fetch(`/api/twitch/profiles/${encodeURIComponent(id)}`, { method: 'DELETE' })
  if (!response.ok) throw new Error((await response.json().catch(() => null))?.error ?? `request failed (${response.status})`)
}

export type TwitchCategory = { id: string; name: string; box_art: string }
export type TwitchStream = {
  login: string; name: string; title: string; game_id: string; game_name: string
  viewers: number; thumb: string; started_at: string | null
}
export type TwitchChannel = { login: string; name: string; live: boolean; title: string; game_name: string; thumb: string }
/** A past broadcast (`kind: 'vod'`, `id` its number) or a clip (`kind: 'clip'`, `id` its slug); `length` in seconds. */
export type TwitchRecording = {
  kind: 'vod' | 'clip'; id: string; title: string; game_name: string; views: number; length: number
  created_at: string | null; thumb: string; resume_position?: number; completed?: boolean
}
export type TwitchChannelPage = { login: string; name: string; live: TwitchStream | null; items: TwitchRecording[]; cursor: string | null }
export type TwitchPage<T> = { items: T[]; cursor: string | null }
const twitchQuery = (params: Record<string, string | undefined>, refresh: boolean) => {
  const query = new URLSearchParams()
  for (const [key, value] of Object.entries(params)) if (value) query.set(key, value)
  if (refresh) query.set('refresh', 'true')
  const text = query.toString()
  return text ? `?${text}` : ''
}
/** `unofficial` (Twitch's web API, no registration) or `official` (Helix with a registered application). */
/** Sign in to Twitch in this profile's own Firefox window (the phone keyboard works as for YouTube). */
export const twitchSignin = (id: string) => playback<LoginStatus>(`twitch/profiles/${encodeURIComponent(id)}/signin`, {})
export const twitchLoginStatus = (id: string, check = false) =>
  playback<LoginStatus>(`twitch/profiles/${encodeURIComponent(id)}/status${check ? '?check=true' : ''}`)
/** The phone link for a Twitch sign-in; the phone page then words its steps for Twitch. */
export const startTwitchPairing = () => playback<Pairing>('pairing?service=twitch', {})
/** Live channels the active profile follows (needs a signed-in Twitch profile). */
export const twitchFollowing = (refresh = false) =>
  playback<TwitchPage<TwitchStream>>(`twitch/following${twitchQuery({}, refresh)}`)
/** Watch a live channel in the CouchTube window; the page to show comes from `playerStatus()`. */
export const twitchPlay = (login: string) => playback<{ status: string }>('twitch/play', { login })
/** A past broadcast from the active profile's saved position (or `start` seconds), or a clip. */
export const twitchPlayVideo = (video: string, start?: number) => playback<{ status: string }>('twitch/play', { video, start })
export const twitchPlayClip = (clip: string) => playback<{ status: string }>('twitch/play', { clip })
/** A channel: its live stream (if on) and its past broadcasts (`vods`) or most watched clips. */
export const twitchChannel = (login: string, kind: 'vods' | 'clips', cursor?: string, refresh = false) =>
  playback<TwitchChannelPage>(`twitch/channel/${encodeURIComponent(login)}${twitchQuery({ kind, cursor }, refresh)}`)
export const twitchStatus = () => playback<{ source: 'official' | 'unofficial' }>('twitch/status')
export const twitchCategories = (cursor?: string, refresh = false) =>
  playback<TwitchPage<TwitchCategory>>(`twitch/categories${twitchQuery({ cursor }, refresh)}`)
/** Live streams of one category, or the most watched on all of Twitch. */
export const twitchStreams = (gameId?: string, cursor?: string, refresh = false) =>
  playback<TwitchPage<TwitchStream>>(`twitch/streams${twitchQuery({ game_id: gameId, cursor }, refresh)}`)
export const twitchSearch = (q: string, cursor?: string, refresh = false) =>
  playback<TwitchPage<TwitchChannel>>(`twitch/search${twitchQuery({ q, cursor }, refresh)}`)

let profileScope = 'guest'
export function setProfileScope(id: string) {
  // Drop pending responses from the previous account as well as its browser cache.
  // The backend retains its own per-profile cache for instant reuse after switching back.
  cache.clear()
  profileScope = id
}

export type FeedKind = 'foryou' | 'continue' | 'local' | 'recommended' | 'subscriptions' | 'history' | 'watchlater' | 'liked'
export const feed = (kind: FeedKind, refresh = false, page = 0) =>
  playback<Video[]>(`feed/${kind}?page=${page}${refresh ? '&refresh=true' : ''}`)
export type Seed = { id: number; kind: 'theme' | 'video' | 'channel' | 'exclude'; value: string; label: string }
export const seeds = () => playback<Seed[]>('seeds')
export const addSeed = (kind: 'theme' | 'video' | 'channel' | 'exclude', value: string, label?: string) =>
  playback<Seed[]>('seeds', { kind, value, label })
export async function removeSeed(id: number) {
  const res = await fetch(`/api/seeds/${id}`, { method: 'DELETE' })
  const body = await res.json().catch(() => null)
  if (!res.ok) throw new Error(body?.error ?? `request failed (${res.status})`)
  return body as Seed[]
}
export const setWatched = (id: string, watched: boolean) => playback<{ watched: boolean }>('watched', { id, watched })
export const dismiss = (id: string) => playback<{ status: string }>('foryou/dismiss', { id })
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

// ---- Music mode -----------------------------------------------------------------------------

/** Music profiles are their own list; they share nothing with the YouTube or Twitch profiles. */
export type MusicProfile = { id: string; name: string; color: string }
export type MusicTrack = {
  id: string; title: string; artist: string | null
  /** Seconds. */
  duration: number | null
  thumb: string
  state: 'offline' | 'missing' | 'downloading' | 'failed'
  progress: number | null
  error: string | null
  favorite: boolean
}
export type MusicPlaylist = { id: number; name: string; count: number }
export type MusicHomeData = { profile: MusicProfile; playlists: MusicPlaylist[]; favorites: number; recent: MusicTrack[] }
export type MusicSearchResult = { kind: 'songs' | 'playlist'; title: string | null; items: MusicTrack[]; more: boolean }
export type MusicState = {
  playing: boolean; track: MusicTrack | null; next: MusicTrack | null
  position: number | null; duration: number | null; paused: boolean; loaded: boolean
  volume: number; muted: boolean; shuffle: boolean; repeat: 'off' | 'all' | 'one'; error: string | null
}
export type MusicLyrics = { found: boolean; synced: [number, string][] | null; plain: string | null }
export type MusicSettings = { cache_limit: number; limits: number[]; files: number; bytes: number }
export type MusicControl =
  | { action: 'pause' | 'resume' | 'toggle' | 'next' | 'prev' | 'mute' | 'volume_up' | 'volume_down' | 'stop' }
  | { action: 'seek' | 'seek_to' | 'jump' | 'volume'; value: number }
  | { action: 'shuffle'; value?: number }
  | { action: 'repeat'; name?: 'off' | 'all' | 'one' }
export type MusicPlay = { playlist_id: number; start?: number; shuffle?: boolean }
  | { favorites: true; start?: number; shuffle?: boolean }
  | { recent: true; start?: number }
  | { video_ids: string[]; start?: number }

async function music<T>(method: 'GET' | 'POST' | 'PUT' | 'DELETE', path: string, body?: object): Promise<T> {
  const res = await fetch(`/api/music/${path}`, {
    method, cache: 'no-store',
    ...(body === undefined ? {} : { headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) }),
  })
  const data = await res.json().catch(() => null)
  if (!res.ok) throw new Error(data?.error ?? `request failed (${res.status})`)
  return data as T
}
const id = encodeURIComponent

export const musicProfiles = () => music<{ profiles: MusicProfile[]; active_id: string }>('GET', 'profiles')
export const createMusicProfile = (name: string, color: string) => music<MusicProfile>('POST', 'profiles', { name, color })
export const selectMusicProfile = (profile: string) => music<MusicProfile>('POST', `profiles/${id(profile)}/select`, {})
export const removeMusicProfile = (profile: string) => music<null>('DELETE', `profiles/${id(profile)}`)
export const musicHome = () => music<MusicHomeData>('GET', 'home')
/** `count` is how many results to show (15, 30, … up to 150). A pasted playlist link lists its songs. */
export const musicSearch = (q: string, count = 15) => music<MusicSearchResult>('GET', `search?q=${id(q)}&count=${count}`)
export const musicPlaylist = (playlist: number) =>
  music<{ id: number; name: string; count: number; items: MusicTrack[] }>('GET', `playlists/${playlist}`)
export const createMusicPlaylist = (name: string) => music<MusicPlaylist>('POST', 'playlists', { name })
export const renameMusicPlaylist = (playlist: number, name: string) => music<{ id: number; name: string }>('PUT', `playlists/${playlist}`, { name })
export const deleteMusicPlaylist = (playlist: number) => music<null>('DELETE', `playlists/${playlist}`)
export const addToMusicPlaylist = (playlist: number, videoIds: string[], importing = false) =>
  music<{ added: number }>('POST', `playlists/${playlist}/tracks`, { video_ids: videoIds, import: importing })
export const removeFromMusicPlaylist = (playlist: number, position: number) => music<null>('DELETE', `playlists/${playlist}/tracks/${position}`)
export const moveInMusicPlaylist = (playlist: number, from: number, to: number) => music<null>('POST', `playlists/${playlist}/move`, { from, to })
export const musicFavorites = () => music<{ items: MusicTrack[] }>('GET', 'favorites')
export const setMusicFavorite = (video: string, on: boolean) => music<null>(on ? 'PUT' : 'DELETE', `favorites/${id(video)}`)
export const musicPlay = (request: MusicPlay) => music<MusicState>('POST', 'play', request)
export const musicControl = (request: MusicControl) => music<MusicState>('POST', 'control', request)
export const musicState = () => music<MusicState>('GET', 'state')
export const musicQueue = () => music<{ items: MusicTrack[] }>('GET', 'queue')
export const musicLyrics = (video: string) => music<MusicLyrics>('GET', `lyrics/${id(video)}`)
export const retryMusicDownload = (video: string) => music<null>('POST', `downloads/${id(video)}`, {})
export const cancelMusicDownload = (video: string) => music<null>('DELETE', `downloads/${id(video)}`)
export const clearMusicDownloads = () => music<{ removed: number }>('DELETE', 'downloads')
export const musicSettings = () => music<MusicSettings>('GET', 'settings')
export const setMusicCacheLimit = (cacheLimit: number) => music<MusicSettings>('PUT', 'settings', { cache_limit: cacheLimit })
export const musicPlaylists = () => music<{ playlists: MusicPlaylist[] }>('GET', 'playlists').then((r) => r.playlists)
