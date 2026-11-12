import { useEffect, useRef, useState } from 'react'
import * as api from './api'
import { formatDuration, lyricLine } from './format'
import { spatial, type Direction } from './nav'
import MusicBar, { MUSIC_COLOR } from './MusicBar'
import MusicRemote from './MusicRemote'

type Screen =
  | { kind: 'home' }
  | { kind: 'search' }
  | { kind: 'playlist'; id: number; name: string }
  | { kind: 'favorites' }
  | { kind: 'queue' }
  | { kind: 'lyrics' }
  | { kind: 'settings' }
  | { kind: 'remote' }
type Entry = { screen: Screen; focus: number }
type Overlay =
  | { kind: 'picker'; ids: string[]; importing: boolean; label: string; lists: api.MusicPlaylist[] | null; creating: boolean }
  | { kind: 'prompt'; title: string; initial: string; submit: (name: string) => Promise<void> }
  | { kind: 'confirm'; title: string; text: string; confirmLabel: string; action: () => Promise<void> }
/** What a screen shows once loaded. */
type Loaded = {
  home?: api.MusicHomeData; items?: api.MusicTrack[]; name?: string
  search?: { kind: 'songs' | 'playlist'; title: string | null; more: boolean }; settings?: api.MusicSettings
}

const SEARCH_STEP = 15
const SEEK_SECONDS = 10
const HOME_MENU: { label: string; note: string; screen: Screen | 'profiles' | 'service' }[] = [
  { label: 'Search', note: 'Find songs, or paste a playlist link', screen: { kind: 'search' } },
  { label: 'Favorites', note: '', screen: { kind: 'favorites' } },
  { label: 'Queue', note: 'What plays next', screen: { kind: 'queue' } },
  { label: 'Now playing', note: 'Lyrics', screen: { kind: 'lyrics' } },
  { label: 'Settings', note: 'Downloads', screen: { kind: 'settings' } },
  { label: 'Phone remote', note: 'Skip and volume from a phone', screen: { kind: 'remote' } },
  { label: 'Profiles', note: 'Switch to another listener', screen: 'profiles' },
  { label: 'Switch service', note: 'Back to YouTube or Twitch', screen: 'service' },
]
const TITLES: Record<string, string> = {
  home: 'Music', search: 'Search songs', favorites: 'Favorites', queue: 'Queue', lyrics: 'Now playing', settings: 'Settings', remote: 'Phone remote',
}
const LAST_PLAYLIST = 'couchtube.music.lastPlaylist'

async function load(screen: Screen, query: string, count: number): Promise<Loaded> {
  switch (screen.kind) {
    case 'home': return { home: await api.musicHome() }
    case 'playlist': { const p = await api.musicPlaylist(screen.id); return { items: p.items, name: p.name } }
    case 'favorites': return { items: (await api.musicFavorites()).items }
    case 'queue': return { items: (await api.musicQueue()).items }
    case 'settings': return { settings: await api.musicSettings() }
    case 'search': {
      if (!query) return { items: [] }
      const r = await api.musicSearch(query, count)
      return { items: r.items, search: { kind: r.kind, title: r.title, more: r.more } }
    }
    default: return {}
  }
}

const screenKey = (screen: Screen, query: string) =>
  screen.kind === 'playlist' ? `playlist:${screen.id}` : screen.kind === 'search' ? `search:${query}` : screen.kind
const titleOf = (screen: Screen) => (screen.kind === 'playlist' ? screen.name : TITLES[screen.kind])
const navButtons = (root: HTMLElement | null) => Array.from(root?.querySelectorAll<HTMLButtonElement>('[data-nav]:not(:disabled)') ?? [])
const songs = (count: number) => `${count} song${count === 1 ? '' : 's'}`

function rememberPlaylist(id: number) { try { localStorage.setItem(LAST_PLAYLIST, String(id)) } catch { /* optional */ } }
function lastPlaylist(): number | null {
  try { const value = Number(localStorage.getItem(LAST_PLAYLIST)); return Number.isInteger(value) && value > 0 ? value : null } catch { return null }
}

const stateLabel = (t: api.MusicTrack) =>
  t.state === 'offline' ? 'Offline' : t.state === 'downloading' ? `Downloading${t.progress !== null ? ` ${Math.round(t.progress * 100)}%` : '…'}`
    : t.state === 'failed' ? 'Failed · R retries' : 'Not downloaded · R'

/** The Music screens after a profile is chosen. The player bar and the playback keys work on all of them. */
export default function MusicHome({ profile, onProfile, onService }: { profile: api.MusicProfile; onProfile: () => void; onService: () => void }) {
  const [stack, setStack] = useState<Entry[]>([{ screen: { kind: 'home' }, focus: 0 }])
  const [home, setHome] = useState<api.MusicHomeData | null>(null)
  const [items, setItems] = useState<api.MusicTrack[]>([])
  const [name, setName] = useState('')
  const [searchInfo, setSearchInfo] = useState<Loaded['search'] | null>(null)
  const [settings, setSettings] = useState<api.MusicSettings | null>(null)
  const [draft, setDraft] = useState('')
  const [query, setQuery] = useState('')
  const [count, setCount] = useState(SEARCH_STEP)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [note, setNote] = useState<string | null>(null)
  const [revision, setRevision] = useState(0)
  const [applied, setApplied] = useState(0)
  const [now, setNow] = useState<api.MusicState | null>(null)
  const [overlay, setOverlay] = useState<Overlay | null>(null)
  const [lyrics, setLyrics] = useState<{ id: string; data: api.MusicLyrics | null; error: string | null } | null>(null)
  const panel = useRef<HTMLElement>(null)
  const overlayPanel = useRef<HTMLDivElement>(null)
  const request = useRef(0)
  const pending = useRef<{ n: number; track: boolean } | null>(null)
  const starting = useRef(false)

  const entry = stack[stack.length - 1]
  const screen = entry.screen
  const key = screenKey(screen, query)
  const isList = screen.kind === 'search' || screen.kind === 'playlist' || screen.kind === 'favorites' || screen.kind === 'queue'
  const rows = screen.kind === 'home' ? home?.recent ?? [] : items
  const nowId = now?.track?.id ?? null

  const focusIndex = () => Math.max(0, navButtons(panel.current).indexOf(document.activeElement as HTMLButtonElement))

  function apply(loaded: Loaded) {
    if (loaded.home) setHome(loaded.home)
    if (loaded.items) setItems(loaded.items)
    if (loaded.name !== undefined) setName(loaded.name)
    if (loaded.search !== undefined) setSearchInfo(loaded.search)
    if (loaded.settings) setSettings(loaded.settings)
    setApplied((n) => n + 1)
  }

  // Load the screen when it opens (or the search text changes, or a refresh is asked for).
  useEffect(() => {
    const mine = ++request.current
    setError(null)
    setHome(null); setItems([]); setSettings(null); setSearchInfo(null)
    if (screen.kind === 'search' && !query) { setLoading(false); setApplied((n) => n + 1); return }
    setLoading(true)
    load(screen, query, SEARCH_STEP).then((loaded) => { if (request.current === mine) { setCount(SEARCH_STEP); apply(loaded) } },
      (err: Error) => { if (request.current === mine) setError(err.message) })
      .finally(() => { if (request.current === mine) setLoading(false) })
    // `entry.focus` is only the position to return to; it must not reload the screen.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, revision])

  /** Reloads the open screen in place (rows keep their place, so the focus stays on its button). */
  async function reload(focus?: { n: number; track: boolean }) {
    const mine = ++request.current
    try {
      const loaded = await load(screen, query, count)
      if (request.current !== mine) return
      if (focus) pending.current = focus
      apply(loaded)
    } catch (err) { if (request.current === mine) setNote((err as Error).message) }
  }

  // Focus: the search box when it opens, otherwise the place that was asked for or the remembered one.
  const lyricsReady = lyrics?.data ? lyrics.id : ''
  useEffect(() => {
    if (overlay || (loading && screen.kind !== 'search')) return
    if (screen.kind === 'search' && !query) { panel.current?.querySelector<HTMLInputElement>('input')?.focus(); return }
    const buttons = navButtons(panel.current)
    if (pending.current) {
      const { n, track } = pending.current
      pending.current = null
      const list = track ? buttons.filter((b) => b.dataset.track) : buttons
      list[Math.min(n, list.length - 1)]?.focus()
    } else if (!panel.current?.contains(document.activeElement) || document.activeElement === panel.current) buttons[entry.focus]?.focus()
    // Focus only when a screen finishes loading.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, applied, lyricsReady])

  // Follow the player: once a second (twice a second where lyrics are timed).
  const fast = screen.kind === 'lyrics'
  useEffect(() => {
    let cancelled = false
    let timer: number
    async function poll() {
      try { const state = await api.musicState(); if (!cancelled) setNow(state) } catch { /* the next poll tries again */ }
      if (!cancelled) timer = window.setTimeout(poll, fast ? 500 : 1000)
    }
    void poll()
    return () => { cancelled = true; window.clearTimeout(timer) }
  }, [fast])

  // Leaving the Music screens stops the music.
  useEffect(() => () => { void api.musicControl({ action: 'stop' }).catch(() => {}) }, [])

  // The queue follows the song; lists with a download running show its progress.
  useEffect(() => { if (screen.kind === 'queue' && nowId) void reload() }, [nowId]) // eslint-disable-line react-hooks/exhaustive-deps
  const downloading = rows.some((t) => t.state === 'downloading')
  const refresh = useRef(reload)
  useEffect(() => { refresh.current = reload })
  const watching = downloading && (isList || screen.kind === 'home') && !overlay
  useEffect(() => {
    if (!watching) return
    const timer = window.setInterval(() => { void refresh.current() }, 2500)
    return () => window.clearInterval(timer)
  }, [watching])

  // Lyrics of the song that plays, fetched when the Now playing screen shows a new song.
  useEffect(() => {
    if (screen.kind !== 'lyrics' || !nowId) return
    if (lyrics?.id === nowId) return
    let cancelled = false
    setLyrics({ id: nowId, data: null, error: null })
    api.musicLyrics(nowId).then((data) => { if (!cancelled) setLyrics({ id: nowId, data, error: null }) },
      (err: Error) => { if (!cancelled) setLyrics({ id: nowId, data: null, error: err.message }) })
    return () => { cancelled = true }
  }, [screen.kind, nowId]) // eslint-disable-line react-hooks/exhaustive-deps
  const synced = lyrics?.id === nowId ? lyrics?.data?.synced ?? null : null
  const currentLyric = synced && now?.position != null ? lyricLine(synced, now.position) : -1
  useEffect(() => {
    if (screen.kind !== 'lyrics' || currentLyric < 0) return
    if (panel.current?.querySelector('.m-lyrics')?.contains(document.activeElement)) return
    panel.current?.querySelector('.m-lyric.current')?.scrollIntoView({ block: 'center', behavior: 'smooth' })
  }, [screen.kind, currentLyric])

  // A note goes away by itself.
  useEffect(() => {
    if (!note) return
    const timer = window.setTimeout(() => setNote(null), 4000)
    return () => window.clearTimeout(timer)
  }, [note])

  // Overlay: focus its first control when it opens.
  const overlayForm = overlay ? `${overlay.kind}:${overlay.kind === 'picker' ? `${overlay.creating}:${overlay.lists === null}` : ''}` : ''
  useEffect(() => {
    if (!overlay) return
    const target = overlayPanel.current?.querySelector<HTMLElement>('input, [data-overlay]')
    target?.focus()
    // Only when the overlay opens or changes its form.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [overlayForm])

  function push(next: Screen) {
    const focus = focusIndex()
    setNote(null)
    setStack((s) => [...s.slice(0, -1), { ...s[s.length - 1], focus }, { screen: next, focus: 0 }])
  }
  function back() {
    setNote(null)
    if (stack.length === 1) onProfile()
    else {
      if (screen.kind === 'search') { setQuery(''); setDraft('') }
      setStack((s) => s.slice(0, -1))
    }
  }
  function openMenu(target: Screen | 'profiles' | 'service') {
    if (target === 'profiles') onProfile()
    else if (target === 'service') onService()
    else push(target)
  }
  /** Goes to a screen from anywhere without piling up copies of it. */
  function show(target: Screen) {
    if (screen.kind === target.kind) return
    push(target)
  }

  async function control(request: api.MusicControl) {
    try { setNow(await api.musicControl(request)) } catch (err) { setNote((err as Error).message) }
  }

  async function playFrom(position: number) {
    if (starting.current) return
    starting.current = true
    setNote('Starting…')
    try {
      let state: api.MusicState
      switch (screen.kind) {
        case 'home': state = await api.musicPlay({ recent: true, start: position }); break
        case 'search': state = await api.musicPlay({ video_ids: items.map((t) => t.id), start: position }); break
        case 'playlist': state = await api.musicPlay({ playlist_id: screen.id, start: position }); break
        case 'favorites': state = await api.musicPlay({ favorites: true, start: position }); break
        case 'queue': state = await api.musicControl({ action: 'jump', value: position }); break
        default: return
      }
      setNow(state)
      setNote(null)
    } catch (err) { setNote((err as Error).message) }
    finally { starting.current = false }
  }

  async function playAll(shuffle: boolean) {
    if (starting.current) return
    starting.current = true
    setNote(shuffle ? 'Shuffling…' : 'Starting…')
    try {
      if (screen.kind === 'playlist') setNow(await api.musicPlay({ playlist_id: screen.id, shuffle }))
      else if (screen.kind === 'favorites') setNow(await api.musicPlay({ favorites: true, shuffle }))
      else if (screen.kind === 'search') setNow(await api.musicPlay({ video_ids: items.map((t) => t.id) }))
      setNote(null)
    } catch (err) { setNote((err as Error).message) }
    finally { starting.current = false }
  }

  async function toggleFavorite(track: api.MusicTrack, position: number | null) {
    const on = !track.favorite
    try { await api.setMusicFavorite(track.id, on) } catch (err) { setNote((err as Error).message); return }
    const patch = (t: api.MusicTrack) => (t.id === track.id ? { ...t, favorite: on } : t)
    setItems((list) => list.map(patch))
    setHome((h) => h && { ...h, recent: h.recent.map(patch), favorites: Math.max(0, h.favorites + (on ? 1 : -1)) })
    setNow((state) => state && { ...state, track: state.track && patch(state.track) })
    setNote(on ? `Added “${track.title}” to Favorites` : `Removed “${track.title}” from Favorites`)
    if (screen.kind === 'favorites' && !on) void reload({ n: position ?? 0, track: true })
  }

  async function removeFromPlaylist(position: number) {
    if (screen.kind !== 'playlist') return
    try { await api.removeFromMusicPlaylist(screen.id, position) } catch (err) { setNote((err as Error).message); return }
    await reload({ n: position, track: true })
  }

  async function moveInPlaylist(position: number, by: -1 | 1) {
    if (screen.kind !== 'playlist') return
    const to = position + by
    if (to < 0 || to >= items.length) return
    try { await api.moveInMusicPlaylist(screen.id, position, to) } catch (err) { setNote((err as Error).message); return }
    await reload({ n: to, track: true })
  }

  async function retryDownload(track: api.MusicTrack) {
    if (track.state === 'offline') return
    try { await api.retryMusicDownload(track.id); setNote(`Downloading “${track.title}”…`); await reload() } catch (err) { setNote((err as Error).message) }
  }

  // ---- overlays ---------------------------------------------------------------------------

  function addTo(ids: string[], label: string, importing = false) {
    if (ids.length === 0) return
    setOverlay({ kind: 'picker', ids, importing, label, lists: null, creating: false })
    api.musicPlaylists().then((lists) => {
      const last = lastPlaylist()
      lists.sort((a, b) => Number(b.id === last) - Number(a.id === last))
      setOverlay((o) => (o?.kind === 'picker' ? { ...o, lists } : o))
    }, (err: Error) => { setOverlay(null); setNote(err.message) })
  }

  async function addToList(list: api.MusicPlaylist, overlayState: Extract<Overlay, { kind: 'picker' }>) {
    try {
      const result = await api.addToMusicPlaylist(list.id, overlayState.ids, overlayState.importing)
      rememberPlaylist(list.id)
      setOverlay(null)
      setNote(`Added ${songs(result.added)} to ${list.name}`)
      if (screen.kind === 'home' || (screen.kind === 'playlist' && screen.id === list.id)) void reload()
    } catch (err) { setNote((err as Error).message); setOverlay(null) }
  }

  async function createAndAdd(text: string, overlayState: Extract<Overlay, { kind: 'picker' }>) {
    try {
      const list = await api.createMusicPlaylist(text)
      await addToList(list, overlayState)
    } catch (err) { setNote((err as Error).message); setOverlay(null) }
  }

  function newPlaylist() {
    setOverlay({
      kind: 'prompt', title: 'New playlist', initial: '',
      submit: async (text) => { const list = await api.createMusicPlaylist(text); rememberPlaylist(list.id); setOverlay(null); setNote(`Created ${list.name}`); await reload() },
    })
  }

  function renamePlaylist() {
    if (screen.kind !== 'playlist') return
    const target = screen
    setOverlay({
      kind: 'prompt', title: 'Rename playlist', initial: name || target.name,
      submit: async (text) => {
        const result = await api.renameMusicPlaylist(target.id, text)
        setOverlay(null)
        setName(result.name)
        setStack((s) => s.map((e) => (e.screen.kind === 'playlist' && e.screen.id === target.id ? { ...e, screen: { ...e.screen, name: result.name } } : e)))
      },
    })
  }

  function deletePlaylist() {
    if (screen.kind !== 'playlist') return
    const target = screen
    setOverlay({
      kind: 'confirm', title: `Delete “${name || target.name}”?`, confirmLabel: 'Delete playlist',
      text: 'The songs stay available to search and play; only the playlist goes.',
      action: async () => { await api.deleteMusicPlaylist(target.id); setOverlay(null); back() },
    })
  }

  function clearDownloads() {
    setOverlay({
      kind: 'confirm', title: 'Clear downloads?', confirmLabel: 'Clear downloads',
      text: 'Downloaded songs are deleted from this machine. They stay in their playlists and stream instead.',
      action: async () => { const r = await api.clearMusicDownloads(); setOverlay(null); setNote(`Removed ${songs(r.removed)}`); await reload() },
    })
  }

  async function setLimit(limit: number) {
    try { setSettings(await api.setMusicCacheLimit(limit)); setNote(`Keeping the newest ${limit} downloads`) } catch (err) { setNote((err as Error).message) }
  }

  // ---- keys -------------------------------------------------------------------------------

  function overlayKeyDown(event: KeyboardEvent) {
    if (event.key === 'Escape' || (event.key === 'Backspace' && !(event.target instanceof HTMLInputElement))) { event.preventDefault(); setOverlay(null); return }
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return
    event.preventDefault()
    const controls = Array.from(overlayPanel.current?.querySelectorAll<HTMLElement>('input, [data-overlay]:not(:disabled)') ?? [])
    const index = controls.indexOf(document.activeElement as HTMLElement)
    controls[Math.max(0, Math.min(controls.length - 1, index + (event.key === 'ArrowDown' ? 1 : -1)))]?.focus()
  }

  function onKeyDown(event: KeyboardEvent) {
    if (event.ctrlKey || event.altKey || event.metaKey) return
    if (overlay) { overlayKeyDown(event); return }
    const typing = event.target instanceof HTMLInputElement
    if (event.key === 'Escape' || (event.key === 'Backspace' && !typing)) { event.preventDefault(); back(); return }
    if (typing) {
      if (event.key === 'ArrowDown') { event.preventDefault(); navButtons(panel.current)[0]?.focus() }
      return
    }
    const focused = document.activeElement instanceof HTMLElement && document.activeElement.dataset.track ? document.activeElement : null
    const position = focused ? Number(focused.dataset.pos) : -1
    const track = position >= 0 ? rows[position] : undefined
    const arrow = ({ ArrowLeft: 'left', ArrowRight: 'right', ArrowUp: 'up', ArrowDown: 'down' } as Record<string, Direction>)[event.key]
    if (arrow) {
      event.preventDefault()
      if (event.shiftKey && (arrow === 'left' || arrow === 'right')) { void control({ action: 'seek', value: arrow === 'left' ? -SEEK_SECONDS : SEEK_SECONDS }); return }
      if (event.shiftKey && screen.kind === 'playlist' && track) { void moveInPlaylist(position, arrow === 'up' ? -1 : 1); return }
      const buttons = navButtons(panel.current)
      const index = buttons.indexOf(document.activeElement as HTMLButtonElement)
      const boxes = buttons.map((b) => { const r = b.getBoundingClientRect(); return { left: r.left, top: r.top, width: r.width, height: r.height } })
      let target = spatial(boxes, index, arrow)
      // Up from the first result returns to the search box.
      if (screen.kind === 'search' && target === index && arrow === 'up') { panel.current?.querySelector<HTMLInputElement>('input')?.focus(); return }
      if (target < 0) target = 0
      buttons[target]?.focus()
      return
    }
    switch (event.key.toLowerCase()) {
      case ' ': event.preventDefault(); if (now?.track) void control({ action: 'toggle' }); return
      case 'n': void control({ action: 'next' }); return
      case 'b': void control({ action: 'prev' }); return
      case '9': void control({ action: 'volume_down' }); return
      case '0': void control({ action: 'volume_up' }); return
      case 'm': void control({ action: 'mute' }); return
      case 's': void control({ action: 'shuffle' }); return
      case 'e': void control({ action: 'repeat' }); return
      case 'l': show({ kind: 'lyrics' }); return
      case 'q': show({ kind: 'queue' }); return
      case 'p': onProfile(); return
      case 'home': if (stack.length > 1) setStack((s) => s.slice(0, 1)); return
      case '/': event.preventDefault(); show({ kind: 'search' }); return
      case 'a': {
        const song = track ?? now?.track
        if (song) addTo([song.id], song.title)
        return
      }
      case 'f': {
        const song = track ?? now?.track
        if (song) void toggleFavorite(song, track ? position : null)
        return
      }
      case 'r': if (track) void retryDownload(track); return
      case 'd':
        if (track && screen.kind === 'playlist') void removeFromPlaylist(position)
        else if (track && screen.kind === 'favorites') void toggleFavorite(track, position)
        return
    }
  }
  // Space also "clicks" a focused button when it is released; the music must pause instead.
  function onKeyUp(event: KeyboardEvent) {
    if (event.key === ' ' && !(event.target instanceof HTMLInputElement) && !overlay) event.preventDefault()
  }
  useEffect(() => {
    window.addEventListener('keydown', onKeyDown)
    window.addEventListener('keyup', onKeyUp)
    return () => { window.removeEventListener('keydown', onKeyDown); window.removeEventListener('keyup', onKeyUp) }
  })

  // ---- screens ----------------------------------------------------------------------------

  const row = (t: api.MusicTrack, position: number) => (
    <button type="button" data-nav data-track={t.id} data-pos={position} key={`${position}:${t.id}`}
      className={`m-row${nowId === t.id ? ' playing' : ''}`} onClick={() => void playFrom(position)}>
      <span className="m-index">{nowId === t.id ? '▶' : screen.kind === 'playlist' ? position + 1 : ''}</span>
      <img className="m-art" src={t.thumb} alt="" loading="lazy" onError={(e) => { e.currentTarget.style.visibility = 'hidden' }} />
      <span className="m-title"><strong>{t.title}</strong><span>{t.artist ?? 'Unknown artist'}</span></span>
      <span className={`m-state ${t.state}`} title={t.error ?? undefined}>{stateLabel(t)}</span>
      <span className="m-fav">{t.favorite ? '♥' : ''}</span>
      <span className="m-dur">{t.duration !== null ? formatDuration(t.duration) : ''}</span>
    </button>
  )

  const importedPlaylist = screen.kind === 'search' && searchInfo?.kind === 'playlist'
  const menuNote = (item: (typeof HOME_MENU)[number]) => (item.label === 'Favorites' ? (home ? songs(home.favorites) : '') : item.note)
  const empty = !loading && !error && rows.length === 0 && (screen.kind !== 'search' || query)
  const emptyText = screen.kind === 'search' ? 'No songs found.' : screen.kind === 'queue' ? 'Nothing is queued. Play a playlist or a search result first.'
    : screen.kind === 'favorites' ? 'No favorites yet. Press F on a song to add it.' : screen.kind === 'playlist' ? 'This playlist is empty. Search for a song and press A to add it.' : ''

  return (
    <main className="app profiles-page music-page" ref={panel}>
      <header className="top"><div className="brand"><span className="logo" style={{ color: MUSIC_COLOR }}>♪</span> CouchTube · Music</div>
        <div className="profile-label" style={{ borderColor: profile.color }}>{profile.name}</div></header>
      <h1 className="page-title">{titleOf(screen)}</h1>

      {screen.kind === 'home' && <>
        <div className="profile-grid m-menu">
          {HOME_MENU.map((item) => <button type="button" data-nav key={item.label} className="profile-card" onClick={() => openMenu(item.screen)}>
            <strong>{item.label}</strong><span className="profile-note">{menuNote(item)}</span></button>)}
        </div>
        {error && <p className="status error" role="alert">{error} <button data-nav onClick={() => setRevision((r) => r + 1)}>Retry</button></p>}
        {home && <>
          <h2 className="m-section">Playlists</h2>
          <div className="profile-grid m-lists">
            {home.playlists.map((list) => <button type="button" data-nav key={list.id} className="profile-card"
              onClick={() => push({ kind: 'playlist', id: list.id, name: list.name })}>
              <strong>{list.name}</strong><span className="profile-note">{songs(list.count)}</span></button>)}
            <button type="button" data-nav className="profile-card" onClick={newPlaylist}>
              <strong>+ New playlist</strong><span className="profile-note">Name it, then add songs from Search</span></button>
          </div>
          {home.recent.length > 0 && <>
            <h2 className="m-section">Recently played</h2>
            <div className="m-list">{home.recent.map(row)}</div>
          </>}
        </>}
        {loading && <p className="status">Loading…</p>}
      </>}

      {screen.kind === 'search' && <>
        <form className="profile-form" onSubmit={(event) => {
          event.preventDefault()
          const text = draft.trim()
          if (!text) return
          pending.current = { n: 0, track: true }
          setCount(SEARCH_STEP)
          setQuery(text)
          setRevision((r) => r + 1)
        }}>
          <input className="search-box" value={draft} maxLength={200} placeholder="Search for a song, or paste a YouTube playlist link" aria-label="Search for songs"
            onChange={(event) => setDraft(event.target.value)} />
        </form>
        {loading && <p className="status">Searching… this can take a few seconds.</p>}
        {importedPlaylist && !loading && <div className="profile-actions">
          <h2 className="m-section">{searchInfo?.title} · {songs(items.length)}</h2>
          <button type="button" data-nav onClick={() => void playAll(false)}>Play all</button>
          <button type="button" data-nav onClick={() => addTo(items.map((t) => t.id), `${songs(items.length)} from ${searchInfo?.title ?? 'the playlist'}`, true)}>Add all to a playlist…</button>
        </div>}
      </>}

      {screen.kind === 'playlist' && <div className="profile-actions m-actions">
        <button type="button" data-nav disabled={items.length === 0} onClick={() => void playAll(false)}>Play</button>
        <button type="button" data-nav disabled={items.length === 0} onClick={() => void playAll(true)}>Shuffle</button>
        <button type="button" data-nav onClick={renamePlaylist}>Rename</button>
        <button type="button" data-nav onClick={deletePlaylist}>Delete</button>
        <span className="m-count">{songs(items.length)}</span>
      </div>}

      {screen.kind === 'favorites' && <div className="profile-actions m-actions">
        <button type="button" data-nav disabled={items.length === 0} onClick={() => void playAll(false)}>Play</button>
        <button type="button" data-nav disabled={items.length === 0} onClick={() => void playAll(true)}>Shuffle</button>
        <span className="m-count">{songs(items.length)}</span>
      </div>}

      {isList && <>
        {error && <p className="status error" role="alert">{error} <button data-nav onClick={() => setRevision((r) => r + 1)}>Retry</button></p>}
        {loading && screen.kind !== 'search' && items.length === 0 && <p className="status">Loading…</p>}
        {empty && <p className="status">{emptyText}</p>}
        <div className="m-list">{items.map(row)}</div>
        {screen.kind === 'search' && searchInfo?.more && !loading && <div className="profile-actions">
          <button type="button" data-nav onClick={() => {
            const before = items.length
            const next = Math.min(150, count + SEARCH_STEP)
            setLoading(true)
            api.musicSearch(query, next).then((r) => {
              setCount(next)
              pending.current = { n: before, track: true }
              apply({ items: r.items, search: { kind: r.kind, title: r.title, more: r.more } })
            }, (err: Error) => setNote(err.message)).finally(() => setLoading(false))
          }}>Load more</button>
        </div>}
      </>}

      {screen.kind === 'lyrics' && <>
        {!now?.track ? <p className="status">Nothing is playing. Start a song, then press <kbd>L</kbd> to see its lyrics.</p> : <>
          <div className="m-now">
            <img src={now.track.thumb} alt="" onError={(e) => { e.currentTarget.style.visibility = 'hidden' }} />
            <div><h2>{now.track.title}</h2><p>{now.track.artist ?? 'Unknown artist'}</p></div>
          </div>
          <div className="m-lyrics">
            {lyrics?.id === nowId && lyrics.error && <p className="status error" role="alert">{lyrics.error}</p>}
            {(lyrics?.id !== nowId || (!lyrics.data && !lyrics.error)) && <p className="status">Looking up the lyrics…</p>}
            {lyrics?.id === nowId && lyrics.data && !lyrics.data.found && <p className="status">No lyrics found.</p>}
            {synced && synced.map(([time, text], index) => <button type="button" data-nav key={`${index}:${time}`}
              className={`m-lyric${index === currentLyric ? ' current' : ''}`} onClick={() => void control({ action: 'seek_to', value: time })}>{text || '♪'}</button>)}
            {!synced && lyrics?.id === nowId && lyrics.data?.plain && <pre className="m-plain">{lyrics.data.plain}</pre>}
          </div>
        </>}
      </>}

      {screen.kind === 'settings' && (settings ? <>
        <p className="status">{songs(settings.files)} downloaded ({(settings.bytes / 1_000_000).toFixed(0)} MB). Only the newest ones are kept; older songs stream again.</p>
        <div className="profile-grid">
          {settings.limits.map((limit) => <button type="button" data-nav key={limit} className="profile-card" onClick={() => void setLimit(limit)}>
            <strong>Keep {limit}</strong><span className="profile-note">{limit === settings.cache_limit ? '✓ In use' : 'downloaded songs'}</span></button>)}
          <button type="button" data-nav className="profile-card" onClick={clearDownloads}>
            <strong>Clear downloads</strong><span className="profile-note">Free the disk space</span></button>
        </div>
      </> : <p className="status">{error ?? 'Loading…'}</p>)}

      {screen.kind === 'remote' && <MusicRemote onNote={setNote} />}

      {overlay && <div className="m-overlay" role="dialog" aria-modal="true" ref={overlayPanel}>
        <div className="m-dialog">
          {overlay.kind === 'picker' && <>
            <h2>Add {overlay.label.includes('from') || overlay.ids.length > 1 ? overlay.label : `“${overlay.label}”`} to…</h2>
            {overlay.creating ? <form className="profile-form" onSubmit={(event) => {
              event.preventDefault()
              const text = String(new FormData(event.currentTarget).get('name') ?? '').trim()
              if (text) void createAndAdd(text, overlay)
            }}>
              <input name="name" className="search-box" maxLength={60} placeholder="Playlist name" aria-label="Playlist name" autoFocus />
              <button type="submit" data-overlay>Create and add</button>
              <button type="button" data-overlay onClick={() => setOverlay({ ...overlay, creating: false })}>Back</button>
            </form> : overlay.lists === null ? <p className="status">Loading…</p> : <div className="m-picks">
              {overlay.lists.map((list) => <button type="button" data-overlay key={list.id} onClick={() => void addToList(list, overlay)}>
                {list.name} <span className="profile-note">{songs(list.count)}</span></button>)}
              <button type="button" data-overlay onClick={() => setOverlay({ ...overlay, creating: true })}>+ New playlist…</button>
              <button type="button" data-overlay onClick={() => setOverlay(null)}>Cancel</button>
            </div>}
          </>}
          {overlay.kind === 'prompt' && <>
            <h2>{overlay.title}</h2>
            <form className="profile-form" onSubmit={(event) => {
              event.preventDefault()
              const text = String(new FormData(event.currentTarget).get('name') ?? '').trim()
              if (text) overlay.submit(text).catch((err: Error) => setNote(err.message))
            }}>
              <input name="name" className="search-box" maxLength={60} defaultValue={overlay.initial} placeholder="Playlist name" aria-label="Playlist name" autoFocus />
              <button type="submit" data-overlay>Save</button>
              <button type="button" data-overlay onClick={() => setOverlay(null)}>Cancel</button>
            </form>
          </>}
          {overlay.kind === 'confirm' && <>
            <h2>{overlay.title}</h2>
            <p className="status">{overlay.text}</p>
            <div className="m-picks">
              <button type="button" data-overlay onClick={() => setOverlay(null)}>Keep it</button>
              <button type="button" data-overlay onClick={() => overlay.action().catch((err: Error) => { setOverlay(null); setNote(err.message) })}>{overlay.confirmLabel}</button>
            </div>
          </>}
        </div>
      </div>}

      {note && <p className="status m-note" role="status">{note}</p>}
      <MusicBar now={now} />
      <footer className="hints"><span><kbd>← ↑ ↓ →</kbd>Move</span><span><kbd>Enter</kbd>{screen.kind === 'home' ? 'Open' : 'Play'}</span><span><kbd>Esc</kbd>Back</span>
        <span><kbd>Space</kbd>Pause</span><span><kbd>⇧← ⇧→</kbd>Seek</span><span><kbd>N</kbd><kbd>B</kbd>Next / back</span>
        <span><kbd>9</kbd><kbd>0</kbd>Volume</span><span><kbd>S</kbd>Shuffle</span><span><kbd>E</kbd>Repeat</span>
        {rows.length > 0 && <><span><kbd>A</kbd>Add</span><span><kbd>F</kbd>Favorite</span></>}
        {screen.kind === 'playlist' && <><span><kbd>D</kbd>Remove</span><span><kbd>⇧↑ ⇧↓</kbd>Move song</span></>}
        <span><kbd>/</kbd>Search</span><span><kbd>Q</kbd>Queue</span><span><kbd>L</kbd>Lyrics</span><span><kbd>P</kbd>Profiles</span></footer>
    </main>
  )
}
