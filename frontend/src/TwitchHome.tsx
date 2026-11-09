import { useEffect, useRef, useState } from 'react'
import * as api from './api'
import { formatAge, formatDuration, formatViews } from './format'

type Screen =
  | { kind: 'home' }
  | { kind: 'following' }
  | { kind: 'live' }
  | { kind: 'categories' }
  | { kind: 'category'; id: string; name: string }
  | { kind: 'search' }
  | { kind: 'remote' }
  | { kind: 'channel'; login: string; name: string; tab: 'vods' | 'clips' }
type Entry = { screen: Screen; focus: number }
type Target =
  | Screen
  | { kind: 'watch'; login: string; name: string }
  | { kind: 'vod'; id: string; name: string; resume?: number }
  | { kind: 'clip'; id: string; name: string }
/** One card in a grid: a stream, a category, a channel, a broadcast or a clip. `channel` is where **C** goes. */
type Card = {
  key: string; art: string; portrait: boolean; title: string; meta: string; live: boolean; target: Target
  channel?: { login: string; name: string }; badge?: { text: string; watched: boolean }; length?: string
}

/** Cards per row: category art is portrait, so more of it fits. */
const columnsFor = (screen: Screen) => (screen.kind === 'categories' ? 6 : 4)
const FOLLOWING_ITEM = { label: 'Following', note: 'Live channels you follow', screen: { kind: 'following' } as Screen }
const HOME_MENU = [
  { label: 'Top live streams', note: 'The most watched right now', screen: { kind: 'live' } as Screen },
  { label: 'Categories', note: 'Browse by game or topic', screen: { kind: 'categories' } as Screen },
  { label: 'Search', note: 'Find a channel', screen: { kind: 'search' } as Screen },
  { label: 'Phone remote', note: 'Pause, volume and chat from a phone', screen: { kind: 'remote' } as Screen },
]
const viewers = (count: number) => `${new Intl.NumberFormat('en', { notation: 'compact' }).format(count)} watching`

const categoryCard = (c: api.TwitchCategory): Card => ({
  key: `c${c.id}`, art: c.box_art, portrait: true, title: c.name, meta: '', live: false,
  target: { kind: 'category', id: c.id, name: c.name },
})
const streamCard = (s: api.TwitchStream): Card => ({
  key: `s${s.login}`, art: s.thumb, portrait: false, title: s.title || s.name,
  meta: [s.name, s.game_name, viewers(s.viewers)].filter(Boolean).join(' · '), live: true,
  target: { kind: 'watch', login: s.login, name: s.name }, channel: { login: s.login, name: s.name },
})
const channelCard = (c: api.TwitchChannel): Card => ({
  key: `h${c.login}`, art: c.thumb, portrait: false, title: c.name,
  meta: c.live ? [c.game_name, c.title].filter(Boolean).join(' · ') : 'Offline', live: c.live,
  // A channel opens its own screen: live stream first, then its broadcasts and clips.
  target: { kind: 'channel', login: c.login, name: c.name, tab: 'vods' }, channel: { login: c.login, name: c.name },
})
const recordingCard = (r: api.TwitchRecording, channel: { login: string; name: string }): Card => ({
  key: `${r.kind}${r.id}`, art: r.thumb, portrait: false, title: r.title || (r.kind === 'clip' ? 'Clip' : 'Broadcast'),
  meta: [r.game_name, formatViews(r.views), formatAge(r.created_at ? Date.parse(r.created_at) / 1000 : null)].filter(Boolean).join(' · '),
  live: false, length: formatDuration(r.length),
  badge: r.completed ? { text: '✓ Watched', watched: true }
    : r.resume_position !== undefined ? { text: `Resume ${formatDuration(r.resume_position)}`, watched: false } : undefined,
  target: r.kind === 'clip' ? { kind: 'clip', id: r.id, name: channel.name } : { kind: 'vod', id: r.id, name: channel.name, resume: r.resume_position },
  channel,
})

async function fetchPage(screen: Screen, query: string, cursor: string | undefined, refresh: boolean) {
  switch (screen.kind) {
    case 'categories': { const p = await api.twitchCategories(cursor, refresh); return { cards: p.items.map(categoryCard), cursor: p.cursor } }
    case 'category': { const p = await api.twitchStreams(screen.id, cursor, refresh); return { cards: p.items.map(streamCard), cursor: p.cursor } }
    case 'following': {
      const p = await api.twitchFollowing(refresh)
      return { cards: p.items.map(streamCard), cursor: p.cursor }
    }
    case 'live': { const p = await api.twitchStreams(undefined, cursor, refresh); return { cards: p.items.map(streamCard), cursor: p.cursor } }
    case 'search': {
      if (!query) return { cards: [], cursor: null }
      const p = await api.twitchSearch(query, cursor, refresh)
      return { cards: p.items.map(channelCard), cursor: p.cursor }
    }
    case 'channel': {
      const p = await api.twitchChannel(screen.login, screen.tab, cursor, refresh)
      const who = { login: p.login, name: p.name }
      // The live stream comes first, on the first page only.
      const live = p.live && !cursor ? [{ ...streamCard(p.live), key: 'live' }] : []
      return { cards: [...live, ...p.items.map((r) => recordingCard(r, who))], cursor: p.cursor }
    }
    default: return { cards: [], cursor: null }
  }
}

const screenKey = (screen: Screen, query: string) =>
  screen.kind === 'category' ? `category:${screen.id}` : screen.kind === 'channel' ? `channel:${screen.login}:${screen.tab}` : screen.kind === 'search' ? `search:${query}` : screen.kind
const titleOf = (screen: Screen) =>
  ({ home: 'Twitch', remote: 'Phone remote', following: 'Following', live: 'Top live streams', categories: 'Categories', search: 'Search Twitch' }[screen.kind as string]
    ?? (screen.kind === 'category' ? screen.name : screen.kind === 'channel' ? `${screen.name} · ${screen.tab === 'vods' ? 'Past broadcasts' : 'Top clips'}` : ''))

/** The Twitch screens after a profile is chosen: Home, Top live, Categories, a category, Search. */
export default function TwitchHome({ profile, onProfile, onService }: { profile: api.TwitchProfile; onProfile: () => void; onService: () => void }) {
  const [stack, setStack] = useState<Entry[]>([{ screen: { kind: 'home' }, focus: 0 }])
  const [cards, setCards] = useState<Card[]>([])
  const [cursor, setCursor] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [note, setNote] = useState<string | null>(null)
  const [draft, setDraft] = useState('')
  const [query, setQuery] = useState('')
  const [revision, setRevision] = useState(0)
  const panel = useRef<HTMLElement>(null)
  const request = useRef(0)
  const refreshNext = useRef(false)
  const focusAfter = useRef<number | null>(null)
  // Watching: the player page covers this screen, like on the YouTube side.
  const [playback, setPlayback] = useState<api.PlayerStatus | null>(null)
  const [playbackError, setPlaybackError] = useState<string | null>(null)
  const [starting, setStarting] = useState(false)
  const startingRef = useRef(false)
  const playbackRevision = useRef(0)
  const focusBeforePlay = useRef(0)
  const wasPlaying = useRef(false)
  const prevPlaying = useRef(false)

  const entry = stack[stack.length - 1]
  const screen = entry.screen
  const key = screenKey(screen, query)
  const isGrid = screen.kind !== 'home' && screen.kind !== 'remote'
  const [remote, setRemote] = useState<api.RemoteInfo | null>(null)

  // Load the first page whenever the screen (or the search text, or a refresh) changes.
  useEffect(() => {
    if (!isGrid) { setCards([]); setCursor(null); return }
    const mine = ++request.current
    const refresh = refreshNext.current
    refreshNext.current = false
    setLoading(true)
    setError(null)
    setCards([])
    setCursor(null)
    fetchPage(screen, query, undefined, refresh).then((page) => {
      if (request.current !== mine) return
      setCards(page.cards)
      setCursor(page.cursor)
      focusAfter.current = Math.min(entry.focus, Math.max(0, page.cards.length - 1)) + (screen.kind === 'search' ? 1 : 0)
    }, (err: Error) => { if (request.current === mine) setError(err.message) })
      .finally(() => { if (request.current === mine) setLoading(false) })
    // `entry.focus` is only the position to return to; it must not reload the screen.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, revision])

  // The remote screen shows the pairing QR code and watches for a phone saying hello (as on the YouTube side).
  useEffect(() => {
    if (screen.kind !== 'remote') { setRemote(null); return }
    let cancelled = false
    let timer: number
    async function poll() {
      try { const info = await api.remoteInfo(); if (!cancelled) setRemote(info) } catch { /* keep the last state */ }
      if (!cancelled) timer = window.setTimeout(poll, 1000)
    }
    void poll()
    return () => { cancelled = true; window.clearTimeout(timer) }
  }, [screen.kind])

  const remoteItems = !remote?.enabled ? [{ label: 'Turn on', action: 'remote-on' }]
    : remote.code && !remote.confirmed ? [{ label: 'Yes, that’s my phone', action: 'remote-yes' }, { label: 'No', action: 'remote-no' }]
    : [{ label: remote.url ? 'New link' : 'Pair a phone', action: 'remote-pair' },
      ...(remote.devices > 0 ? [{ label: `Forget ${remote.devices} phone${remote.devices === 1 ? '' : 's'}`, action: 'remote-forget' }] : []),
      { label: 'Turn off', action: 'remote-off' }]

  async function remoteAction(action: string) {
    try {
      if (action === 'remote-on') setRemote(await api.setRemote(true))
      else if (action === 'remote-off') setRemote(await api.setRemote(false))
      else if (action === 'remote-pair') setRemote(await api.remotePair())
      else if (action === 'remote-forget') { await api.forgetPhones(); setRemote(await api.remoteInfo()) }
      else if (action === 'remote-yes' && remote?.code) { await api.remoteConfirm(remote.code); setRemote(await api.remoteInfo()) }
      else if (action === 'remote-no') { await api.remoteReject(); setRemote(await api.remotePair()) }
    } catch (err) { setNote((err as Error).message) }
  }

  // The buttons on the remote screen change with its state: keep the focus on one of them.
  const remoteLabels = screen.kind === 'remote' && remote ? remoteItems.map((i) => i.label).join('|') : ''
  useEffect(() => {
    if (screen.kind !== 'remote' || !remoteLabels) return
    if (!panel.current?.contains(document.activeElement) || document.activeElement === panel.current) navButtons()[0]?.focus()
    // navButtons only reads the DOM.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [remoteLabels])

  // Focus: the search box when it opens, otherwise the remembered card.
  useEffect(() => {
    if (screen.kind === 'search' && !query) { panel.current?.querySelector<HTMLInputElement>('input')?.focus(); return }
    const buttons = panel.current?.querySelectorAll<HTMLButtonElement>('[data-nav]:not(:disabled)')
    if (!isGrid) buttons?.[entry.focus]?.focus()
    else if (focusAfter.current !== null) { buttons?.[focusAfter.current]?.focus(); focusAfter.current = null }
    // Focus only when the screen or its cards change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, cards.length, loading])

  const navButtons = () => Array.from(panel.current?.querySelectorAll<HTMLButtonElement>('[data-nav]:not(:disabled)') ?? [])
  const focusIndex = () => Math.max(0, navButtons().indexOf(document.activeElement as HTMLButtonElement))

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
  function open(target: Target) {
    if (target.kind === 'watch') void startStream(() => api.twitchPlay(target.login), `twitch:${target.login}`, target.name)
    else if (target.kind === 'vod') void startStream(() => api.twitchPlayVideo(target.id), `twitch:vod:${target.id}`, target.name)
    else if (target.kind === 'clip') void startStream(() => api.twitchPlayClip(target.id), `twitch:clip:${target.id}`, target.name)
    else push(target)
  }
  /** Switches the open channel between its past broadcasts and its clips. */
  function switchTab() {
    if (screen.kind !== 'channel') return
    const tab = screen.tab === 'vods' ? 'clips' : 'vods'
    setNote(null)
    setStack((s) => [...s.slice(0, -1), { screen: { ...screen, tab }, focus: 0 }])
  }
  async function startStream(begin: () => Promise<unknown>, id: string, name: string) {
    if (startingRef.current) return
    startingRef.current = true
    playbackRevision.current++
    focusBeforePlay.current = focusIndex()
    setStarting(true)
    setPlaybackError(null)
    setNote(`Starting ${name}…`)
    try {
      await begin()
      wasPlaying.current = true
      setPlayback({ playing: true, id, title: name, position: null, duration: null, paused: false, error: null })
    } catch (err) {
      setPlaybackError((err as Error).message)
      setNote(null)
    } finally {
      startingRef.current = false
      setStarting(false)
    }
  }

  async function stopStream() {
    playbackRevision.current++
    try {
      await api.stop()
      setPlayback(null)
      setPlaybackError(null)
    } catch (err) { setPlaybackError((err as Error).message) }
  }

  /** A stream that failed (for example an offline channel) leaves its message until a key is pressed. */
  function dismissError() {
    setPlaybackError(null)
    void api.stop().catch(() => {})
  }

  function loadMore() {
    if (!cursor || loading) return
    const mine = ++request.current
    const before = cards.length
    setLoading(true)
    fetchPage(screen, query, cursor, false).then((page) => {
      if (request.current !== mine) return
      setCards((current) => [...current, ...page.cards.filter((c) => !current.some((o) => o.key === c.key))])
      setCursor(page.cursor)
      focusAfter.current = before
    }, (err: Error) => { if (request.current === mine) setError(err.message) })
      .finally(() => { if (request.current === mine) setLoading(false) })
  }

  // Follow the backend's player: it tells us which page to show and when the stream ended.
  useEffect(() => {
    let cancelled = false
    let timer: number
    async function poll() {
      const revision = playbackRevision.current
      try {
        const status = await api.playerStatus()
        if (!cancelled && revision === playbackRevision.current && !startingRef.current) {
          // Only Twitch sessions belong to this screen.
          const ours = !status.playing || (status.id ?? '').startsWith('twitch:')
          if (ours) {
            wasPlaying.current = status.playing
            setPlayback(status)
            setPlaybackError(status.error)
            if (status.playing) setNote(null)
          }
        }
      } catch { /* the next poll tries again */ }
      if (!cancelled) timer = window.setTimeout(poll, 1000)
    }
    void poll()
    return () => { cancelled = true; window.clearTimeout(timer) }
  }, [])

  // Leaving the Twitch screens stops whatever is playing.
  useEffect(() => () => { void api.stop().catch(() => {}) }, [])

  // When the stream ends, the focus returns to the card it was started from.
  const isPlaying = Boolean(playback?.playing)
  useEffect(() => {
    if (!isPlaying && prevPlaying.current) navButtons()[focusBeforePlay.current]?.focus()
    prevPlaying.current = isPlaying
    // navButtons only reads the DOM.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isPlaying])

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      // While a stream plays, Esc stops it (also when the page does not have the focus).
      if (starting || playback?.playing) {
        if (event.key === 'Escape' || event.key === 'Backspace') { event.preventDefault(); if (!starting) void stopStream() }
        else if (event.key.startsWith('Arrow') || event.key === 'Enter' || event.key === '/') event.preventDefault()
        return
      }
      if (playbackError) {
        dismissError()
        if (event.key === 'Escape') { event.preventDefault(); return }
      }
      const typing = event.target instanceof HTMLInputElement
      if (event.key === 'Escape' || (event.key === 'Backspace' && !typing)) { event.preventDefault(); back(); return }
      if (typing) {
        if (event.key === 'ArrowDown') { event.preventDefault(); navButtons()[0]?.focus() }
        return
      }
      const offset = ({ ArrowLeft: -1, ArrowRight: 1, ArrowUp: -columnsFor(screen), ArrowDown: columnsFor(screen) } as Record<string, number>)[event.key]
      if (offset !== undefined) {
        event.preventDefault()
        const buttons = navButtons()
        const index = buttons.indexOf(document.activeElement as HTMLButtonElement)
        // Up from the first row of results returns to the search box.
        if (screen.kind === 'search' && index >= 0 && index + offset < 0) { panel.current?.querySelector<HTMLInputElement>('input')?.focus(); return }
        buttons[Math.max(0, Math.min(buttons.length - 1, index + offset))]?.focus()
      } else if (event.key === 'u' && isGrid) { refreshNext.current = true; setRevision((r) => r + 1) }
      else if (event.key === 't' && screen.kind === 'channel') switchTab()
      else if (event.key === 'c' && isGrid) {
        const index = focusIndex()
        const channel = cards[index]?.channel
        if (channel && !(screen.kind === 'channel' && screen.login === channel.login)) push({ kind: 'channel', login: channel.login, name: channel.name, tab: 'vods' })
      }
      else if (event.key === '/' && screen.kind !== 'search') { event.preventDefault(); push({ kind: 'search' }) }
      else if (event.key === 'p') onProfile()
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  })

  return (
    <main className="app profiles-page" ref={panel}>
      <header className="top"><div className="brand"><span className="logo" style={{ color: '#9146ff' }}>▶</span> CouchTube · Twitch</div>
        <div className="profile-label" style={{ borderColor: profile.color }}>{profile.name}</div></header>
      <h1 className="page-title">{titleOf(screen)}</h1>
      {screen.kind === 'home' && <div className="profile-grid">
        {(profile.id === 'guest' ? HOME_MENU : [FOLLOWING_ITEM, ...HOME_MENU]).map((item) => <button type="button" data-nav key={item.label} className="profile-card" onClick={() => push(item.screen)}>
          <strong>{item.label}</strong><span className="profile-note">{item.note}</span></button>)}
        <button type="button" data-nav className="profile-card" onClick={onService}>
          <strong>Switch service</strong><span className="profile-note">Back to YouTube or Twitch</span></button>
      </div>}
      {screen.kind === 'remote' && remote && <>
        <div className="profile-grid">
          {remoteItems.map((item) => <button type="button" data-nav key={item.action} className="profile-card" onClick={() => void remoteAction(item.action)}>
            <strong>{item.label}</strong></button>)}
        </div>
        <section className="pairing">
          {remote.svg && <img alt="QR code for pairing a phone with the remote" src={`data:image/svg+xml;utf8,${encodeURIComponent(remote.svg)}`} />}
          <div className="pairing-text">
            {!remote.enabled ? <p>The remote is off. Turn it on to control the stream from a phone on this network.</p>
              : remote.code && !remote.confirmed ? <div className="pairing-confirm" role="alert">
                <p>A phone connected. Does it show this code?</p>
                <strong className="pairing-code">{remote.code.slice(0, 3)} {remote.code.slice(3)}</strong>
              </div>
              : remote.url ? <>
                <p>{remote.confirmed ? '✓ Phone paired. Choose New link to pair another.' : 'Scan with your phone (same Wi-Fi). The link works once for 10 minutes.'}</p>
                <p className="pairing-url">{remote.url}</p>
              </>
              : <p>{remote.devices > 0 ? `${remote.devices} phone${remote.devices === 1 ? ' is' : 's are'} paired. Choose Pair a phone to add another.` : 'No phone is paired yet. Choose Pair a phone.'} Use it on a network you trust.</p>}
          </div>
        </section>
      </>}
      {screen.kind === 'search' && <form className="profile-form" onSubmit={(event) => {
        event.preventDefault()
        const text = draft.trim()
        if (text) setQuery(text)
      }}>
        <input className="search-box" value={draft} maxLength={100} placeholder="Search channels" aria-label="Search channels"
          onChange={(event) => setDraft(event.target.value)} />
      </form>}
      {isGrid && <>
        {error && <p className="status error" role="alert">{error} <button data-nav onClick={() => setRevision((r) => r + 1)}>Retry</button></p>}
        {loading && cards.length === 0 && <p className="status">Loading…</p>}
        {!loading && !error && cards.length === 0 && (screen.kind !== 'search' || query) &&
          <p className="status">{screen.kind === 'search' ? 'No channels found.' : screen.kind === 'channel' ? 'No recordings yet.' : screen.kind === 'following' ? 'None of the channels you follow are live right now.' : 'Nothing here right now.'}</p>}
        <div className={`tw-grid ${screen.kind === 'categories' ? 'tw-six' : ''}`}>
          {cards.map((card) => <button type="button" data-nav key={card.key} className="card tw-card" onClick={() => open(card.target)}>
            <div className={`thumb ${card.portrait ? 'tw-portrait' : ''}`}>
              {card.art && <img src={card.art} alt="" loading="lazy" onError={(e) => { e.currentTarget.style.visibility = 'hidden' }} />}
              {card.badge && <span className={`resume-badge ${card.badge.watched ? 'watched-badge' : ''}`}>{card.badge.text}</span>}
              {card.live && <span className="badge live">LIVE</span>}
              {card.length && <span className="badge">{card.length}</span>}
            </div>
            <div className="card-title">{card.title}</div>
            {card.meta && <div className="card-meta">{card.meta}</div>}
          </button>)}
        </div>
        {cursor && <div className="profile-actions"><button data-nav disabled={loading} onClick={loadMore}>{loading ? 'Loading…' : 'Load more'}</button></div>}
      </>}
      {note && <p className="status" role="status">{note}</p>}
      {playbackError && <p className="status error" role="alert">{playbackError}</p>}
      {playback?.playing && playback.page && (
        <iframe
          key={playback.page}
          className="player-frame"
          src={playback.page}
          title="Twitch player"
          allow="autoplay; fullscreen"
          ref={(frame) => frame?.focus()}
          onLoad={(e) => e.currentTarget.contentWindow?.focus()}
        />
      )}
      <footer className="hints"><span><kbd>← ↑ ↓ →</kbd>Move</span><span><kbd>Enter</kbd>Open</span><span><kbd>Esc</kbd>Back</span>
        {screen.kind !== 'search' && <span><kbd>/</kbd>Search</span>}
        {screen.kind === 'channel' && <span><kbd>T</kbd>{screen.tab === 'vods' ? 'Clips' : 'Broadcasts'}</span>}
        {isGrid && screen.kind !== 'channel' && screen.kind !== 'categories' && screen.kind !== 'search' && <span><kbd>C</kbd>Channel</span>}
        {isGrid && <span><kbd>U</kbd>Refresh</span>}
        <span><kbd>P</kbd>Profiles</span></footer>
    </main>
  )
}
