import { useEffect, useMemo, useRef, useState, type RefObject } from 'react'
import * as api from './api'
import type { Video } from './api'
import { formatDuration, formatViews } from './format'
import { clamp, move, START, type Direction, type Focus, type RowSpec } from './nav'
import { addRecent, loadRecent } from './recent'

type Screen =
  | { kind: 'home' }
  | { kind: 'search'; query: string }
  | { kind: 'channel'; id: string; name: string | null }

/** One level of the back stack. Focus is kept per level so Back returns to the same card. */
type Entry = { screen: Screen; focus: Focus }

type Row =
  | { kind: 'search-button' }
  | { kind: 'search-input' }
  | { kind: 'chips'; title: string; items: string[] }
  | { kind: 'videos'; title?: string; videos: Video[]; carousel: boolean }

// Placeholder rows until the account feeds (Phase 5) replace them.
const HOME_ROWS = [
  { title: 'Music', query: 'music mix' },
  { title: 'Nature', query: 'nature documentary 4k' },
  { title: 'Science & tech', query: 'science explained' },
]

const GRID_COLUMNS = 4

const ARROWS: Record<string, Direction> = {
  ArrowUp: 'up',
  ArrowDown: 'down',
  ArrowLeft: 'left',
  ArrowRight: 'right',
}

export default function App() {
  const [stack, setStack] = useState<Entry[]>([{ screen: { kind: 'home' }, focus: START }])
  const [recent, setRecent] = useState(loadRecent)
  const [draft, setDraft] = useState('')
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

  async function startVideo(video: Video) {
    if (startingRef.current) return
    startingRef.current = true
    playbackRevision.current++
    setStarting(true)
    setPlaybackError(null)
    selectedTitle.current = { id: video.id, title: video.title }
    try {
      await api.play(video.id)
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
      setPlaybackError(null)
    } catch (err) { setPlaybackError((err as Error).message) }
  }

  const { screen } = stack[stack.length - 1]
  const key = screenKey(screen)
  const data = useScreenRows(screen)

  const rows = useMemo<Row[]>(() => {
    switch (screen.kind) {
      case 'home':
        return [
          { kind: 'search-button' },
          ...(recent.length > 0
            ? [{ kind: 'chips', title: 'Recent searches', items: recent } as const]
            : []),
          ...data.rows,
        ]
      case 'search':
        return [{ kind: 'search-input' }, ...data.rows]
      case 'channel':
        return data.rows
    }
  }, [screen.kind, recent, data.rows])

  const specs: RowSpec[] = rows.map((row) => ({
    length: rowLength(row),
    remember: row.kind === 'chips' || (row.kind === 'videos' && row.carousel),
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
  }
  const push = (next: Screen, focusAt: Focus = START) =>
    setStack((s) => [...s, { screen: next, focus: focusAt }])
  const replaceTop = (next: Screen, focusAt: Focus) =>
    setStack((s) => [...s.slice(0, -1), { screen: next, focus: focusAt }])
  const back = () => setStack((s) => (s.length > 1 ? s.slice(0, -1) : s))
  const goHome = () => setStack((s) => [s[0]])

  function openSearch() {
    if (screen.kind === 'search') setFocus(START)
    else push({ kind: 'search', query: '' })
  }

  function runSearch(raw: string) {
    const query = raw.trim()
    if (!query) return
    setRecent(addRecent(query))
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

  function activate() {
    switch (focusedRow?.kind) {
      case 'search-button':
        openSearch()
        break
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
        if (e.key === 'Escape' || e.key === 'Backspace') {
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
      </header>

      {screen.kind === 'home' && <h1 className="page-title">What do you want to watch?</h1>}
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

      {data.loading && <SkeletonRow />}
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
        {videoRowFocused && <span><kbd>C</kbd>Channel</span>}
        <span><kbd>/</kbd>Search</span>
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
  const meta = [video.channel, formatViews(video.view_count)].filter(Boolean).join(' · ')
  return (
    <div className={`card ${focused ? 'focused' : ''}`}>
      <div className="thumb">
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
function useScreenRows(screen: Screen) {
  const key = screenKey(screen)
  const [state, setState] = useState<{ key: string; rows: Row[]; error: string | null }>()

  useEffect(() => {
    let cancelled = false
    const done = (rows: Row[], error: string | null = null) => {
      if (!cancelled) setState({ key, rows, error })
    }
    const pending = loadRows(screen)
    if (pending) pending.then((rows) => done(rows), (err: Error) => done([], err.message))
    else done([])
    return () => {
      cancelled = true
    }
    // `key` identifies the screen; the object itself changes identity on every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key])

  const current = state?.key === key ? state : undefined
  return {
    rows: current?.rows ?? [],
    loading: current === undefined,
    error: current?.error ?? null,
  }
}

function loadRows(screen: Screen): Promise<Row[]> | null {
  switch (screen.kind) {
    case 'home':
      return Promise.allSettled(HOME_ROWS.map((r) => api.search(r.query))).then((results) => {
        const rows: Row[] = []
        results.forEach((result, i) => {
          if (result.status === 'fulfilled' && result.value.length > 0) {
            rows.push({ kind: 'videos', title: HOME_ROWS[i].title, videos: result.value, carousel: true })
          }
        })
        if (rows.length === 0) {
          const failure = results.find((r) => r.status === 'rejected')
          if (failure) throw failure.reason
        }
        return rows
      })
    case 'search':
      if (!screen.query) return null
      return api.search(screen.query).then((videos) => gridRows(videos, `Results for “${screen.query}”`))
    case 'channel':
      return api.channel(screen.id).then((channel) => gridRows(channel.videos, 'Latest uploads'))
  }
}

/** Splits videos into rows of GRID_COLUMNS, so ↑/↓ moves between visual rows. */
function gridRows(videos: Video[], title: string): Row[] {
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
    case 'channel':
      return 'Channel'
  }
}
