import { useEffect, useRef, useState } from 'react'
import * as api from './api'

type Screen =
  | { kind: 'home' }
  | { kind: 'live' }
  | { kind: 'categories' }
  | { kind: 'category'; id: string; name: string }
  | { kind: 'search' }
type Entry = { screen: Screen; focus: number }
type Target = Screen | { kind: 'watch'; login: string; name: string } | { kind: 'offline'; name: string }
/** One card in a grid: a stream, a category or a channel. */
type Card = { key: string; art: string; portrait: boolean; title: string; meta: string; live: boolean; target: Target }

/** Cards per row: category art is portrait, so more of it fits. */
const columnsFor = (screen: Screen) => (screen.kind === 'categories' ? 6 : 4)
const HOME_MENU = [
  { label: 'Top live streams', note: 'The most watched right now', screen: { kind: 'live' } as Screen },
  { label: 'Categories', note: 'Browse by game or topic', screen: { kind: 'categories' } as Screen },
  { label: 'Search', note: 'Find a channel', screen: { kind: 'search' } as Screen },
]
const viewers = (count: number) => `${new Intl.NumberFormat('en', { notation: 'compact' }).format(count)} watching`

const categoryCard = (c: api.TwitchCategory): Card => ({
  key: `c${c.id}`, art: c.box_art, portrait: true, title: c.name, meta: '', live: false,
  target: { kind: 'category', id: c.id, name: c.name },
})
const streamCard = (s: api.TwitchStream): Card => ({
  key: `s${s.login}`, art: s.thumb, portrait: false, title: s.title || s.name,
  meta: [s.name, s.game_name, viewers(s.viewers)].filter(Boolean).join(' · '), live: true,
  target: { kind: 'watch', login: s.login, name: s.name },
})
const channelCard = (c: api.TwitchChannel): Card => ({
  key: `h${c.login}`, art: c.thumb, portrait: false, title: c.name,
  meta: c.live ? [c.game_name, c.title].filter(Boolean).join(' · ') : 'Offline', live: c.live,
  target: c.live ? { kind: 'watch', login: c.login, name: c.name } : { kind: 'offline', name: c.name },
})

async function fetchPage(screen: Screen, query: string, cursor: string | undefined, refresh: boolean) {
  switch (screen.kind) {
    case 'categories': { const p = await api.twitchCategories(cursor, refresh); return { cards: p.items.map(categoryCard), cursor: p.cursor } }
    case 'category': { const p = await api.twitchStreams(screen.id, cursor, refresh); return { cards: p.items.map(streamCard), cursor: p.cursor } }
    case 'live': { const p = await api.twitchStreams(undefined, cursor, refresh); return { cards: p.items.map(streamCard), cursor: p.cursor } }
    case 'search': {
      if (!query) return { cards: [], cursor: null }
      const p = await api.twitchSearch(query, cursor, refresh)
      return { cards: p.items.map(channelCard), cursor: p.cursor }
    }
    default: return { cards: [], cursor: null }
  }
}

const screenKey = (screen: Screen, query: string) =>
  screen.kind === 'category' ? `category:${screen.id}` : screen.kind === 'search' ? `search:${query}` : screen.kind
const titleOf = (screen: Screen) =>
  ({ home: 'Twitch', live: 'Top live streams', categories: 'Categories', search: 'Search Twitch' }[screen.kind as string]
    ?? (screen.kind === 'category' ? screen.name : ''))

/** The Twitch screens after a profile is chosen: Home, Top live, Categories, a category, Search. */
export default function TwitchHome({ profile, onProfile, onService }: { profile: api.TwitchProfile; onProfile: () => void; onService: () => void }) {
  const [stack, setStack] = useState<Entry[]>([{ screen: { kind: 'home' }, focus: 0 }])
  const [configured, setConfigured] = useState<boolean | null>(null)
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

  const entry = stack[stack.length - 1]
  const screen = entry.screen
  const key = screenKey(screen, query)
  const isGrid = screen.kind !== 'home'

  useEffect(() => {
    api.twitchStatus().then((s) => setConfigured(s.configured), () => setConfigured(false))
  }, [])

  // Load the first page whenever the screen (or the search text, or a refresh) changes.
  useEffect(() => {
    if (!isGrid || !configured) { setCards([]); setCursor(null); return }
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
  }, [key, configured, revision])

  // Focus: the search box when it opens, otherwise the remembered card.
  useEffect(() => {
    if (!configured) return
    if (screen.kind === 'search' && !query) { panel.current?.querySelector<HTMLInputElement>('input')?.focus(); return }
    const buttons = panel.current?.querySelectorAll<HTMLButtonElement>('[data-nav]:not(:disabled)')
    if (!isGrid) buttons?.[entry.focus]?.focus()
    else if (focusAfter.current !== null) { buttons?.[focusAfter.current]?.focus(); focusAfter.current = null }
    // Focus only when the screen or its cards change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, configured, cards.length, loading])

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
    if (target.kind === 'watch') setNote(`Watching ${target.name} arrives in the next build step.`)
    else if (target.kind === 'offline') setNote(`${target.name} is offline right now.`)
    else push(target)
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

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
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
      {configured === null && <p className="status">Loading…</p>}
      {configured === false && <div className="status" role="status">
        <p>Twitch is not set up yet. Register a free application at <strong>dev.twitch.tv/console</strong> (OAuth redirect URL: <code>http://localhost</code>, category: Application Integration), then start CouchTube with</p>
        <p><code>COUCHTUBE_TWITCH_CLIENT_ID</code> and <code>COUCHTUBE_TWITCH_CLIENT_SECRET</code> set to the application’s client id and secret.</p>
        <div className="profile-actions">
          <button data-nav onClick={onProfile}>Switch profile</button>
          <button data-nav onClick={onService}>Switch service</button>
        </div>
      </div>}
      {configured && screen.kind === 'home' && <div className="profile-grid">
        {HOME_MENU.map((item) => <button type="button" data-nav key={item.label} className="profile-card" onClick={() => push(item.screen)}>
          <strong>{item.label}</strong><span className="profile-note">{item.note}</span></button>)}
        <button type="button" data-nav className="profile-card" onClick={onService}>
          <strong>Switch service</strong><span className="profile-note">Back to YouTube or Twitch</span></button>
      </div>}
      {configured && screen.kind === 'search' && <form className="profile-form" onSubmit={(event) => {
        event.preventDefault()
        const text = draft.trim()
        if (text) setQuery(text)
      }}>
        <input className="search-box" value={draft} maxLength={100} placeholder="Search channels" aria-label="Search channels"
          onChange={(event) => setDraft(event.target.value)} />
      </form>}
      {configured && isGrid && <>
        {error && <p className="status error" role="alert">{error} <button data-nav onClick={() => setRevision((r) => r + 1)}>Retry</button></p>}
        {loading && cards.length === 0 && <p className="status">Loading…</p>}
        {!loading && !error && cards.length === 0 && (screen.kind !== 'search' || query) &&
          <p className="status">{screen.kind === 'search' ? 'No channels found.' : 'Nothing here right now.'}</p>}
        <div className={`tw-grid ${screen.kind === 'categories' ? 'tw-six' : ''}`}>
          {cards.map((card) => <button type="button" data-nav key={card.key} className="card tw-card" onClick={() => open(card.target)}>
            <div className={`thumb ${card.portrait ? 'tw-portrait' : ''}`}>
              {card.art && <img src={card.art} alt="" loading="lazy" onError={(e) => { e.currentTarget.style.visibility = 'hidden' }} />}
              {card.live && <span className="badge live">LIVE</span>}
            </div>
            <div className="card-title">{card.title}</div>
            {card.meta && <div className="card-meta">{card.meta}</div>}
          </button>)}
        </div>
        {cursor && <div className="profile-actions"><button data-nav disabled={loading} onClick={loadMore}>{loading ? 'Loading…' : 'Load more'}</button></div>}
      </>}
      {note && <p className="status" role="status">{note}</p>}
      <footer className="hints"><span><kbd>← ↑ ↓ →</kbd>Move</span><span><kbd>Enter</kbd>Open</span><span><kbd>Esc</kbd>Back</span>
        {configured && screen.kind !== 'search' && <span><kbd>/</kbd>Search</span>}
        {configured && isGrid && <span><kbd>U</kbd>Refresh</span>}
        <span><kbd>P</kbd>Profiles</span></footer>
    </main>
  )
}
