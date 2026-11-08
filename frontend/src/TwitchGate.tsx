import { useEffect, useRef, useState } from 'react'
import * as api from './api'

type View = { kind: 'list' } | { kind: 'add' } | { kind: 'remove'; profile: api.TwitchProfile }
const COLORS = ['#9146ff', '#6288d9', '#c7799e', '#53a888', '#cb9a53', '#dc7474']
const COLOR_NAMES = ['Purple', 'Blue', 'Pink', 'Green', 'Gold', 'Red']

/** "Who's watching?" for Twitch: a list of Twitch profiles, separate from the YouTube ones. */
export default function TwitchGate({ onService }: { onService: () => void }) {
  const [profiles, setProfiles] = useState<api.TwitchProfile[]>([])
  const [active, setActive] = useState<api.TwitchProfile | null>(null)
  const [view, setView] = useState<View>({ kind: 'list' })
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [name, setName] = useState('')
  const [color, setColor] = useState(COLORS[0])
  const [highlighted, setHighlighted] = useState(0)
  const panel = useRef<HTMLElement>(null)
  const operation = useRef(false)

  async function refresh() { setProfiles((await api.twitchProfiles()).profiles) }

  useEffect(() => {
    let cancelled = false
    api.twitchProfiles().then((result) => { if (!cancelled) setProfiles(result.profiles) },
      (err: Error) => { if (!cancelled) setError(err.message) }).finally(() => { if (!cancelled) setLoading(false) })
    return () => { cancelled = true }
  }, [])

  useEffect(() => {
    if (loading || busy) return
    const target = view.kind === 'add'
      ? panel.current?.querySelector<HTMLInputElement>('input')
      : panel.current?.querySelectorAll<HTMLButtonElement>('[data-nav]:not(:disabled)')[view.kind === 'list' ? highlighted : 0]
    target?.focus()
    // Focus only when a view opens, an operation completes or the profile page appears.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [view.kind, active, loading, busy])

  async function perform(action: () => Promise<void>) {
    if (operation.current) return
    operation.current = true
    setBusy(true)
    setError(null)
    try { await action() }
    catch (err) { setError((err as Error).message) }
    finally { operation.current = false; setBusy(false) }
  }

  const choose = (profile: api.TwitchProfile) => perform(async () => {
    await api.selectTwitchProfile(profile.id)
    setActive(profile)
  })

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === 'Escape' || (event.key === 'Backspace' && !(event.target instanceof HTMLInputElement))) {
        event.preventDefault()
        if (busy) return
        setError(null)
        if (active) setActive(null)
        else if (view.kind === 'list') onService()
        else setView({ kind: 'list' })
        return
      }
      if (event.target instanceof HTMLInputElement || event.target instanceof HTMLSelectElement || busy) return
      const buttons = Array.from(panel.current?.querySelectorAll<HTMLButtonElement>('[data-nav]:not(:disabled)') ?? [])
      const index = buttons.indexOf(document.activeElement as HTMLButtonElement)
      const columns = !active && view.kind === 'list' ? 4 : 1
      const offset = ({ ArrowLeft: -1, ArrowRight: 1, ArrowUp: -columns, ArrowDown: columns } as Record<string, number>)[event.key]
      if (offset !== undefined) {
        event.preventDefault()
        buttons[Math.max(0, Math.min(buttons.length - 1, index + offset))]?.focus()
      } else if (!active && view.kind === 'list' && event.key === 'd') {
        const profile = profiles[highlighted]
        if (profile && profile.id !== 'guest') { event.preventDefault(); setView({ kind: 'remove', profile }) }
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  })

  return (
    <main className="app profiles-page" ref={panel}>
      <header className="top"><div className="brand"><span className="logo">▶</span> CouchTube · Twitch</div>
        {active && <div className="profile-label" style={{ borderColor: active.color }}>{active.name}</div>}</header>
      {active && <>
        <h1 className="page-title">Twitch</h1>
        <p className="status">Signed in as the Twitch profile “{active.name}”. Browsing and watching streams arrive in the next build steps (see docs/twitch-mode.md).</p>
        <div className="profile-actions">
          <button data-nav onClick={() => setActive(null)}>Switch profile</button>
          <button data-nav onClick={onService}>Switch service</button>
        </div>
      </>}
      {!active && view.kind === 'list' && <>
        <h1 className="page-title">Who’s watching Twitch?</h1>
        {loading ? <p className="status">Loading profiles…</p> : <div className="profile-grid">
          {profiles.map((profile, index) => <button type="button" data-nav key={profile.id}
            className="profile-card" disabled={busy} onFocus={() => setHighlighted(index)} onClick={() => void choose(profile)}>
            <span className="profile-avatar" style={{ background: profile.color }}>{profile.name.slice(0, 1).toUpperCase()}</span>
            <strong>{profile.name}</strong>
            <span className="profile-note">{profile.id === 'guest' ? 'Browse without an account' : 'Twitch profile'}</span>
          </button>)}
          <button type="button" data-nav className="profile-card" disabled={busy}
            onFocus={() => setHighlighted(profiles.length)} onClick={() => { setName(''); setView({ kind: 'add' }) }}>
            <span className="profile-avatar add-avatar">+</span><strong>Add profile</strong><span className="profile-note">A separate Twitch profile</span>
          </button>
        </div>}
        {!loading && profiles.length === 0 && <button data-nav onClick={() => void perform(refresh)}>Retry</button>}
        <footer className="hints"><span><kbd>← ↑ ↓ →</kbd>Move</span><span><kbd>Enter</kbd>Choose</span>
          <span><kbd>D</kbd>Remove profile</span><span><kbd>Esc</kbd>Change service</span></footer>
      </>}
      {!active && view.kind === 'add' && <>
        <h1 className="page-title">Add a Twitch profile</h1>
        <p className="status">Twitch profiles are separate from YouTube profiles. Signing in to Twitch comes in a later step.</p>
        <form className="profile-form" onSubmit={(event) => {
          event.preventDefault()
          void perform(async () => {
            await api.createTwitchProfile(name, color)
            await refresh()
            setView({ kind: 'list' })
          })
        }}>
          <label>Name<input className="search-box" value={name} maxLength={40} required disabled={busy}
            onChange={(event) => setName(event.target.value)} placeholder="Your name" /></label>
          <label>Colour<select value={color} disabled={busy} onChange={(event) => setColor(event.target.value)}>
            {COLORS.map((value, index) => <option value={value} key={value}>{COLOR_NAMES[index]}</option>)}
          </select></label>
          <button type="submit" disabled={busy || !name.trim()}>Create profile</button>
          <button type="button" disabled={busy} onClick={() => setView({ kind: 'list' })}>Cancel</button>
        </form>
      </>}
      {!active && view.kind === 'remove' && <>
        <h1 className="page-title">Remove {view.profile.name}?</h1>
        <p className="status">This deletes this Twitch profile. Your YouTube profiles are not affected.</p>
        <div className="profile-actions">
          <button data-nav disabled={busy} onClick={() => setView({ kind: 'list' })}>Keep profile</button>
          <button data-nav disabled={busy} onClick={() => void perform(async () => {
            await api.removeTwitchProfile(view.profile.id)
            await refresh()
            setHighlighted(0)
            setView({ kind: 'list' })
          })}>Remove profile</button>
        </div>
      </>}
      {busy && <p className="status" role="status">Please wait…</p>}
      {error && <p className="status error" role="alert">{error}</p>}
    </main>
  )
}
