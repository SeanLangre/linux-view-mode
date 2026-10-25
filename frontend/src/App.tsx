import { useEffect, useMemo, useRef, useState, type RefObject } from 'react'
import * as api from './api'
import type { Video } from './api'
import { formatAge, formatDuration, formatViews } from './format'
import { clamp, move, START, type Direction, type Focus, type RowSpec } from './nav'
import { addRecent, loadRecent } from './recent'

type Screen =
  | { kind: 'home' }
  | { kind: 'search'; query: string }
  | { kind: 'channel'; id: string; name: string | null }
  | { kind: 'feed'; feed: api.FeedKind }
  | { kind: 'queue' }

/** One level of the back stack. Focus is kept per level so Back returns to the same card. */
type Entry = { screen: Screen; focus: Focus }

type Row =
  | { kind: 'search-button' }
  | { kind: 'search-input' }
  | { kind: 'chips'; title: string; items: string[] }
  | { kind: 'menu'; items: { label: string; action: string }[] }
  | { kind: 'videos'; title?: string; videos: Video[]; carousel: boolean; error?: string }

const HOME_MENU = [
  { label: 'Recommended', action: 'recommended' }, { label: 'Subscriptions', action: 'subscriptions' },
  { label: 'History', action: 'history' }, { label: 'Watch later', action: 'watchlater' },
  { label: 'Liked', action: 'liked' }, { label: 'Queue', action: 'queue' },
  { label: 'Exit to desktop', action: 'exit' },
]
const FEED_NAMES: Record<api.FeedKind, string> = {
  continue: 'Continue watching', recommended: 'Recommended', subscriptions: 'Subscriptions',
  history: 'History', watchlater: 'Watch later', liked: 'Liked videos',
}

const GRID_COLUMNS = 4

const ARROWS: Record<string, Direction> = {
  ArrowUp: 'up',
  ArrowDown: 'down',
  ArrowLeft: 'left',
  ArrowRight: 'right',
}

export default function App({ profile, onSwitch }: { profile: api.Profile; onSwitch: () => void }) {
  const [stack, setStack] = useState<Entry[]>([{ screen: { kind: 'home' }, focus: START }])
  const [recent, setRecent] = useState(() => loadRecent(profile.id))
  const [draft, setDraft] = useState('')
  const [dataRevision, setDataRevision] = useState(0)
  const refreshRequested = useRef(false)
  const [feedback, setFeedback] = useState<string | null>(null)
  const feedbackTimer = useRef<number | undefined>(undefined)
  const wasPlaying = useRef(false)
  const inputRef = useRef<HTMLInputElement>(null)
  const [playback, setPlayback] = useState<api.PlayerStatus | null>(null)
  const [playbackError, setPlaybackError] = useState<string | null>(null)
  const [starting, setStarting] = useState(false)
  const startingRef = useRef(false)
  const selectedTitle = useRef<{ id: string; title: string } | null>(null)
  const playbackRevision = useRef(0)

  useEffect(() => {
    let cancelled = false
    let timer: number
    async function poll() {
      const revision = playbackRevision.current
      try {
        const status = await api.playerStatus()
        if (!cancelled && revision === playbackRevision.current && !startingRef.current) {
          if (wasPlaying.current && !status.playing) setDataRevision((value) => value + 1)
          wasPlaying.current = status.playing
          setPlayback(status)
          setPlaybackError(status.error)
        }
      } catch (err) {
        if (!cancelled) setPlaybackError((err as Error).message)
      }
      if (!cancelled) timer = window.setTimeout(poll, 1000)
    }
    void poll()
    return () => { cancelled = true; window.clearTimeout(timer) }
  }, [])

  async function startVideo(video: Video, start?: number) {
    if (startingRef.current) return
    startingRef.current = true
    playbackRevision.current++
    setStarting(true)
    setPlaybackError(null)
    selectedTitle.current = { id: video.id, title: video.title }
    try {
      await api.play(video.id, start)
      wasPlaying.current = true
      setPlayback({ playing: true, id: video.id, title: video.title,
        position: null, duration: null, paused: false, error: null })
    } catch (err) {
      setPlaybackError((err as Error).message)
    } finally {
      startingRef.current = false
      setStarting(false)
    }
  }

  async function stopVideo() {
    playbackRevision.current++
    try {
      await api.stop()
      setPlayback(null)
      wasPlaying.current = false
      setDataRevision((value) => value + 1)
      setPlaybackError(null)
    } catch (err) { setPlaybackError((err as Error).message) }
  }

  const { screen } = stack[stack.length - 1]
  const key = screenKey(screen)
  const data = useScreenRows(screen, dataRevision, refreshRequested)
  const refreshing = data.refreshing

  const rows = useMemo<Row[]>(() => {
    switch (screen.kind) {
      case 'home':
        return [
          { kind: 'search-button' },
          { kind: 'menu', items: HOME_MENU },
          ...data.rows,
          ...(recent.length > 0
            ? [{ kind: 'chips', title: 'Recent searches', items: recent } as const]
            : []),
        ]
      case 'search':
        return [{ kind: 'search-input' }, ...data.rows]
      case 'queue':
        return [{ kind: 'menu', items: [{ label: 'Play next', action: 'next' }, { label: 'Clear queue', action: 'clear' }] }, ...data.rows]
      case 'feed':
      case 'channel':
        return data.rows
    }
  }, [screen.kind, recent, data.rows])

  const specs: RowSpec[] = rows.map((row) => ({
    length: rowLength(row),
    remember: row.kind === 'menu' || row.kind === 'chips' || (row.kind === 'videos' && row.carousel),
  }))
  const focus = clamp(stack[stack.length - 1].focus, specs)
  const focusedRow = rows.length > 0 ? rows[focus.row] : undefined
  const inputFocused = focusedRow?.kind === 'search-input'

  const setFocus = (next: Focus) =>
    setStack((s) => [...s.slice(0, -1), { ...s[s.length - 1], focus: next }])
  // A blocked move (edge of the screen, or rows still loading) must not overwrite
  // the stored focus, which may point at a row that hasn't loaded yet.
  const moveFocus = (dir: Direction) => {
    const next = move(focus, dir, specs)
    if (next.row !== focus.row || next.col !== focus.col) setFocus(next)
    else if (dir === 'down') data.loadMore()
  }
  const push = (next: Screen, focusAt: Focus = START) =>
    setStack((s) => [...s, { screen: next, focus: focusAt }])
  const replaceTop = (next: Screen, focusAt: Focus) =>
    setStack((s) => [...s.slice(0, -1), { screen: next, focus: focusAt }])
  const back = () => {
    if (stack.length === 1) onSwitch()
    else setStack((s) => s.slice(0, -1))
  }
  const goHome = () => setStack((s) => [s[0]])

  function openSearch() {
    if (screen.kind === 'search') setFocus(START)
    else push({ kind: 'search', query: '' })
  }

  function runSearch(raw: string) {
    const query = raw.trim()
    if (!query) return
    setRecent(addRecent(query, profile.id))
    // Focus aims at the first result; it stays on the input until results arrive.
    const target = { ...START, row: 1 }
    // Replace rather than stack searches, so Back always leads out of search.
    if (screen.kind === 'search') replaceTop({ kind: 'search', query }, target)
    else push({ kind: 'search', query }, target)
  }

  function focusedVideo(): Video | undefined {
    return focusedRow?.kind === 'videos' ? focusedRow.videos[focus.col] : undefined
  }

  function openChannel() {
    const video = focusedVideo()
    if (!video?.channel_id) return
    if (screen.kind === 'channel' && screen.id === video.channel_id) return
    push({ kind: 'channel', id: video.channel_id, name: video.channel })
  }

  function notify(message: string) {
    setFeedback(message)
    window.clearTimeout(feedbackTimer.current)
    feedbackTimer.current = window.setTimeout(() => setFeedback(null), 2500)
  }

  async function queueAction(action: 'add' | 'remove' | 'next' | 'clear') {
    try {
      if (action === 'add') {
        const video = focusedVideo()
        if (!video) return
        await api.enqueue(video.id)
        notify(`Queued: ${video.title}`)
      } else if (action === 'remove') {
        const video = focusedVideo()
        if (!video) return
        await api.deleteQueue(video.id)
      } else if (action === 'clear') await api.deleteQueue()
      else { const result = await api.next(); if (!result.playing) notify('Queue is empty') }
      setDataRevision((value) => value + 1)
    } catch (err) { notify((err as Error).message) }
  }

  function activate() {
    switch (focusedRow?.kind) {
      case 'search-button':
        openSearch()
        break
      case 'menu': {
        const action = focusedRow.items[focus.col]?.action
        if (action === 'queue') push({ kind: 'queue' })
        else if (action === 'exit') void api.exitToDesktop().catch((err: Error) => notify(err.message))
        else if (action === 'next' || action === 'clear') void queueAction(action)
        else if (action) push({ kind: 'feed', feed: action as api.FeedKind })
        break
      }
      case 'chips':
        runSearch(focusedRow.items[focus.col])
        break
      case 'videos':
        { const video = focusedVideo(); if (video) void startVideo(video) }
        break
    }
  }

  // Re-registered every render so the handler always sees current state.
  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      // Keep the browsing position until mpv exits, including when Firefox retains focus.
      if (starting || playback?.playing) {
        if (e.key === 'n' && !starting) { e.preventDefault(); void queueAction('next') }
        else if (e.key === 'Escape' || e.key === 'Backspace') {
          e.preventDefault()
          if (!starting) void stopVideo()
        } else if (ARROWS[e.key] || ['Enter', '/', 'Home', 'c'].includes(e.key)) {
          e.preventDefault()
        }
        return
      }
      if (document.activeElement === inputRef.current) {
        // Typing: only leave the input with ↓ or Esc; Enter submits the form.
        if (e.key === 'ArrowDown') {
          e.preventDefault()
          moveFocus('down')
        } else if (e.key === 'Escape') {
          e.preventDefault()
          back()
        }
        return
      }

      const dir = ARROWS[e.key]
      if (dir) {
        e.preventDefault()
        moveFocus(dir)
        return
      }
      switch (e.key) {
        case 'Enter':
          e.preventDefault()
          activate()
          break
        case 'Escape':
        case 'Backspace':
          e.preventDefault()
          back()
          break
        case '/':
          e.preventDefault()
          openSearch()
          break
        case 'Home':
          e.preventDefault()
          goHome()
          break
        case 'a':
          void queueAction('add')
          break
        case 'd':
          if (screen.kind === 'queue') void queueAction('remove')
          break
        case 'n':
          void queueAction('next')
          break
        case 'r': { const video = focusedVideo(); if (video) void startVideo(video, 0); break }
        case 'u':
          refreshRequested.current = true
          setDataRevision((value) => value + 1)
          break
        case 'p':
          onSwitch()
          break
        case 'c':
          openChannel()
          break
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  })

  // Real DOM focus only goes to the search box; everything else is a highlighted card.
  useEffect(() => {
    if (inputFocused) inputRef.current?.focus()
    else if (document.activeElement instanceof HTMLElement) document.activeElement.blur()
  }, [inputFocused, key])

  useEffect(() => {
    if (screen.kind === 'search') setDraft(screen.query)
    // Only when the screen changes, not on every keystroke.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key])

  useEffect(() => {
    document
      .querySelector('.focused')
      ?.scrollIntoView({ block: 'center', inline: 'nearest', behavior: 'smooth' })
  }, [focus.row, focus.col, key, rows.length])

  const videoRowFocused = focusedRow?.kind === 'videos'

  return (
    <main className="app">
      <header className="top">
        <div className="brand">
          <span className="logo">▶</span> CouchTube
        </div>
        <div className="crumb">{crumb(screen)}</div>
        <div className="profile-label" style={{ borderColor: profile.color }}>{profile.name}</div>
      </header>

      {screen.kind === 'home' && <h1 className="page-title">What do you want to watch?</h1>}
      {screen.kind === 'feed' && <h1 className="page-title">{FEED_NAMES[screen.feed]}</h1>}
      {screen.kind === 'queue' && <h1 className="page-title">Up next</h1>}
      {screen.kind === 'channel' && <h1 className="page-title">{screen.name ?? 'Channel'}</h1>}

      {rows.map((row, r) => (
        <RowView
          key={`${key}:${r}`}
          row={row}
          focusedCol={focus.row === r ? focus.col : -1}
          inputRef={inputRef}
          draft={draft}
          onDraft={setDraft}
          onSubmit={() => runSearch(draft)}
        />
      ))}

      {(data.loading || data.loadingMore) && <SkeletonRow />}
      {!data.loading && !data.error && !data.rows.some((row) => row.kind === 'videos' && row.videos.length > 0) && (
        <p className="status">{screen.kind === 'home' ? 'Search for a video, or sign in to load your recommendations and subscriptions.'
          : screen.kind === 'queue' ? 'Your queue is empty. Press A on a video to add it.'
          : screen.kind === 'feed' ? 'No videos here. Account feeds need a signed-in profile; press U to refresh.' : ''}</p>
      )}
      {data.error && (
        <p className="status error">
          {data.error} · <kbd>Esc</kbd> to go back
        </p>
      )}
      {screen.kind === 'search' && !data.loading && !data.error && (
        <p className="status">
          {screen.query
            ? data.rows.length === 0 && 'No results'
            : 'Type what you want to watch and press Enter.'}
        </p>
      )}

      <footer className="hints">
        <span><kbd>← ↑ ↓ →</kbd>Move</span>
        <span><kbd>Enter</kbd>{videoRowFocused ? 'Play' : 'Select'}</span>
        {videoRowFocused && <span><kbd>A</kbd>Queue</span>}
        {videoRowFocused && <span><kbd>C</kbd>Channel</span>}
        {screen.kind === 'queue' && <span><kbd>D</kbd>Remove</span>}
        <span><kbd>N</kbd>Next</span>
        <span><kbd>U</kbd>Refresh</span>
        <span><kbd>/</kbd>Search</span>
        <span><kbd>P</kbd>Profiles</span>
        {stack.length > 1 && <span><kbd>Esc</kbd>Back</span>}
      </footer>

      {(starting || playback?.playing) && (
        <div className="toast" role="status">
          {starting ? 'Starting' : 'Playing'}: {playback?.title
            ?? (playback?.id === selectedTitle.current?.id ? selectedTitle.current?.title : playback?.id)
            ?? selectedTitle.current?.title}
          {playback?.paused && ' · Paused'}
          {' · Esc to stop'}
        </div>
      )}
      {refreshing && <div className="toast" role="status">Refreshing…</div>}
      {feedback && <div className="toast" role="status">{feedback}</div>}
      {playbackError && <div className="toast error" role="alert">{playbackError}</div>}
    </main>
  )
}

function RowView({
  row,
  focusedCol,
  inputRef,
  draft,
  onDraft,
  onSubmit,
}: {
  row: Row
  focusedCol: number
  inputRef: RefObject<HTMLInputElement | null>
  draft: string
  onDraft: (value: string) => void
  onSubmit: () => void
}) {
  switch (row.kind) {
    case 'search-button':
      return (
        <div className={`search-box fake ${focusedCol === 0 ? 'focused' : ''}`}>
          Search YouTube…
        </div>
      )
    case 'search-input':
      return (
        <form
          onSubmit={(e) => {
            e.preventDefault()
            onSubmit()
          }}
        >
          <input
            ref={inputRef}
            className="search-box"
            value={draft}
            onChange={(e) => onDraft(e.target.value)}
            placeholder="Search YouTube…"
            aria-label="Search YouTube"
            spellCheck={false}
            autoComplete="off"
          />
        </form>
      )
    case 'menu':
      return <div className="chips" aria-label="Browse">{row.items.map((item, i) => <span key={item.action} className={`chip ${i === focusedCol ? 'focused' : ''}`}>{item.label}</span>)}</div>
    case 'chips':
      return (
        <section>
          <h2 className="row-title">{row.title}</h2>
          <div className="chips">
            {row.items.map((item, i) => (
              <span key={item} className={`chip ${i === focusedCol ? 'focused' : ''}`}>
                {item}
              </span>
            ))}
          </div>
        </section>
      )
    case 'videos':
      if (row.videos.length === 0) return row.error ? <p className="status error">{row.title}: {row.error}</p> : null
      return (
        <section>
          {row.title && <h2 className="row-title">{row.title}</h2>}
          <div className={row.carousel ? 'carousel' : 'grid'}>
            {row.videos.map((video, i) => (
              <VideoCard key={video.id} video={video} focused={i === focusedCol} />
            ))}
          </div>
        </section>
      )
  }
}

function VideoCard({ video, focused }: { video: Video; focused: boolean }) {
  const meta = [video.channel, formatViews(video.view_count), formatAge(video.timestamp)].filter(Boolean).join(' · ')
  return (
    <div className={`card ${focused ? 'focused' : ''}`}>
      <div className="thumb">
        {video.resume_position !== undefined && <span className="resume-badge">Resume {formatDuration(video.resume_position)}</span>}
        <img src={video.thumb} alt="" loading="lazy" />
        <span className={`badge ${video.duration === null ? 'live' : ''}`}>
          {formatDuration(video.duration)}
        </span>
      </div>
      <div className="card-title">{video.title}</div>
      <div className="card-meta">{meta}</div>
    </div>
  )
}

function SkeletonRow() {
  return (
    <div className="grid" aria-label="Loading">
      {Array.from({ length: GRID_COLUMNS }, (_, i) => (
        <div key={i} className="card">
          <div className="thumb skeleton" />
          <div className="card-title skeleton-text" />
        </div>
      ))}
    </div>
  )
}

/** Loads the video rows for a screen. Results are cached in `api`, so going back is instant. */
function useScreenRows(screen: Screen, revision: number, refreshRequested: RefObject<boolean>) {
  const key = screenKey(screen)
  type State = { key: string; rows: Row[]; error: string | null; videos?: Video[]; page: number; done: boolean }
  const [state, setState] = useState<State>()
  const [loadingMore, setLoadingMore] = useState(false)
  const [refreshing, setRefreshing] = useState(false)
  const moreBusy = useRef(false)
  const screenRef = useRef(screen)
  screenRef.current = screen

  useEffect(() => {
    let cancelled = false
    const done = (next: Omit<State, 'key'>) => {
      if (!cancelled) setState({ key, ...next })
    }
    const refresh = refreshRequested.current
    refreshRequested.current = false
    moreBusy.current = false
    setLoadingMore(false)
    if (refresh) setRefreshing(true)
    const finish = () => { if (!cancelled) setRefreshing(false) }
    const fail = (err: Error) => { finish(); done({ rows: [], error: err.message, page: 0, done: true }) }
    if (paged(screen)) {
      loadPage(screen, 0, refresh).then((videos) => {
        finish()
        done({ rows: gridRows(videos, pageTitle(screen)), videos, page: 0, error: null,
          done: videos.length < api.PAGE_SIZE })
      }, fail)
    } else {
      const pending = loadRows(screen, refresh)
      if (pending) {
        pending.then((rows) => { finish(); done({ rows, page: 0, error: null, done: true }) }, fail)
      } else done({ rows: [], error: null, page: 0, done: true })
    }
    return () => {
      cancelled = true
      setRefreshing(false)
    }
    // `key` identifies the screen; the object itself changes identity on every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, revision])

  const current = state?.key === key ? state : undefined

  function loadMore() {
    if (!current?.videos || current.done || moreBusy.current) return
    moreBusy.current = true
    setLoadingMore(true)
    const page = current.page + 1
    const known = current.videos
    loadPage(screenRef.current, page, false)
      .then((fetched) => {
        const seen = new Set(known.map((v) => v.id))
        const added = fetched.filter((v) => !seen.has(v.id))
        // Recommended can repeat itself; an empty or short page means there is no more.
        const videos = [...known, ...added]
        setState((s) => s && s.key === key ? { ...s, videos, rows: gridRows(videos, pageTitle(screenRef.current)), page,
          done: fetched.length < api.PAGE_SIZE || added.length === 0 || page + 1 >= api.MAX_PAGES } : s)
      })
      .catch(() => setState((s) => s && s.key === key ? { ...s, done: true } : s))
      .finally(() => { moreBusy.current = false; setLoadingMore(false) })
  }

  return {
    rows: current?.rows ?? [],
    loading: current === undefined,
    loadingMore,
    refreshing,
    loadMore,
    error: current?.error ?? null,
  }
}

/** Screens that load more videos when focus reaches the bottom. */
function paged(screen: Screen): boolean {
  return screen.kind === 'feed' || screen.kind === 'channel'
}

function pageTitle(screen: Screen): string | undefined {
  return screen.kind === 'channel' ? 'Latest uploads' : undefined
}

function loadPage(screen: Screen, page: number, refresh: boolean): Promise<Video[]> {
  if (screen.kind === 'feed') return api.feed(screen.feed, refresh, page)
  if (screen.kind === 'channel') return api.channel(screen.id, page, refresh).then((c) => c.videos)
  return Promise.resolve([])
}

function loadRows(screen: Screen, refresh: boolean): Promise<Row[]> | null {
  switch (screen.kind) {
    case 'home': {
      const kinds = ['continue', 'recommended', 'subscriptions'] as const
      return Promise.allSettled(kinds.map((kind) => api.feed(kind, refresh))).then((results) =>
        results.map((result, index): Row => ({ kind: 'videos', title: FEED_NAMES[kinds[index]], carousel: true,
          videos: result.status === 'fulfilled' ? result.value : [],
          error: result.status === 'rejected' ? (result.reason as Error).message : undefined,
        })))
    }
    case 'queue':
      return api.queue().then((videos) => gridRows(videos))
    case 'search':
      if (!screen.query) return null
      return api.search(screen.query, refresh).then((videos) => gridRows(videos, `Results for “${screen.query}”`))
    default:
      return null
  }
}

/** Splits videos into rows of GRID_COLUMNS, so ↑/↓ moves between visual rows. */
function gridRows(videos: Video[], title?: string): Row[] {
  const rows: Row[] = []
  for (let i = 0; i < videos.length; i += GRID_COLUMNS) {
    rows.push({
      kind: 'videos',
      title: i === 0 ? title : undefined,
      videos: videos.slice(i, i + GRID_COLUMNS),
      carousel: false,
    })
  }
  return rows
}

function rowLength(row: Row): number {
  switch (row.kind) {
    case 'search-button':
    case 'search-input':
      return 1
    case 'menu':
    case 'chips':
      return row.items.length
    case 'videos':
      return row.videos.length
  }
}

function screenKey(screen: Screen): string {
  switch (screen.kind) {
    case 'home':
      return 'home'
    case 'search':
      return `search:${screen.query}`
    case 'queue': return 'queue'
    case 'feed': return `feed:${screen.feed}`
    case 'channel':
      return `channel:${screen.id}`
  }
}

function crumb(screen: Screen): string {
  switch (screen.kind) {
    case 'home':
      return 'Home'
    case 'search':
      return 'Search'
    case 'queue': return 'Queue'
    case 'feed': return FEED_NAMES[screen.feed]
    case 'channel':
      return 'Channel'
  }
}
