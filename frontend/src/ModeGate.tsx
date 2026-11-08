import { useEffect, useRef, useState } from 'react'
import * as api from './api'
import ProfileGate from './ProfileGate'
import TwitchGate from './TwitchGate'

const SERVICES: { mode: api.Mode; name: string; note: string; color: string }[] = [
  { mode: 'youtube', name: 'YouTube', note: 'Videos, subscriptions and your own recommendations', color: '#e0443e' },
  { mode: 'twitch', name: 'Twitch', note: 'Live streams, categories and channels', color: '#9146ff' },
]

/** The first screen: choose YouTube or Twitch. Each service has its own profiles and screens. */
export default function ModeGate() {
  const [mode, setMode] = useState<api.Mode | null>(null)
  const [last, setLast] = useState<api.Mode>('youtube')
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const panel = useRef<HTMLElement>(null)

  useEffect(() => {
    let cancelled = false
    api.getMode().then((saved) => { if (!cancelled && saved) setLast(saved) }, () => {})
      .finally(() => { if (!cancelled) setLoading(false) })
    return () => { cancelled = true }
  }, [])

  // The service used last is highlighted, so the usual case is a single Enter.
  useEffect(() => {
    if (mode || loading) return
    const buttons = panel.current?.querySelectorAll<HTMLButtonElement>('[data-nav]')
    buttons?.[SERVICES.findIndex((service) => service.mode === last)]?.focus()
  }, [mode, loading, last])

  useEffect(() => {
    if (mode) return
    function onKeyDown(event: KeyboardEvent) {
      const offset = ({ ArrowLeft: -1, ArrowRight: 1 } as Record<string, number>)[event.key]
      if (offset === undefined) return
      event.preventDefault()
      const buttons = Array.from(panel.current?.querySelectorAll<HTMLButtonElement>('[data-nav]') ?? [])
      const index = buttons.indexOf(document.activeElement as HTMLButtonElement)
      buttons[Math.max(0, Math.min(buttons.length - 1, index + offset))]?.focus()
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [mode])

  async function choose(next: api.Mode) {
    if (busy) return
    setBusy(true)
    setError(null)
    try {
      await api.setMode(next) // also stops anything that is playing
      setLast(next)
      setMode(next)
    } catch (err) { setError((err as Error).message) }
    finally { setBusy(false) }
  }

  if (mode === 'youtube') return <ProfileGate onService={() => setMode(null)} />
  if (mode === 'twitch') return <TwitchGate onService={() => setMode(null)} />

  return (
    <main className="app profiles-page" ref={panel}>
      <header className="top"><div className="brand"><span className="logo">▶</span> CouchTube</div></header>
      <h1 className="page-title">What are we watching?</h1>
      {loading ? <p className="status">Loading…</p> : <div className="mode-grid">
        {SERVICES.map((service) => <button type="button" data-nav key={service.mode} className="profile-card mode-card"
          disabled={busy} onClick={() => void choose(service.mode)}>
          <span className="profile-avatar" style={{ background: service.color }}>{service.name.slice(0, 1)}</span>
          <strong>{service.name}</strong>
          <span className="profile-note">{service.note}</span>
        </button>)}
      </div>}
      <footer className="hints"><span><kbd>← →</kbd>Move</span><span><kbd>Enter</kbd>Choose</span></footer>
      {error && <p className="status error" role="alert">{error}</p>}
    </main>
  )
}
