import { useEffect, useRef, useState } from 'react'
import * as api from './api'
import App from './App'

type View = { kind: 'list' } | { kind: 'add' } | { kind: 'manage' | 'remove'; profile: api.Profile }
const COLORS = ['#6288d9', '#c7799e', '#53a888', '#cb9a53', '#927bd4', '#dc7474']
const BUSY_STATES = new Set(['signing_in', 'checking'])

export default function ProfileGate() {
  const [profiles, setProfiles] = useState<api.Profile[]>([])
  const [active, setActive] = useState<api.Profile | null>(null)
  const [view, setView] = useState<View>({ kind: 'list' })
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [name, setName] = useState('')
  const [color, setColor] = useState(COLORS[0])
  const [statuses, setStatuses] = useState<Record<string, api.LoginStatus>>({})
  const [highlighted, setHighlighted] = useState(0)
  const panel = useRef<HTMLElement>(null)
  const operation = useRef(false)
  const managedId = !active && view.kind === 'manage' ? view.profile.id : null
  const login = managedId ? statuses[managedId] : undefined
  const [qr, setQr] = useState<api.Pairing | null>(null)
  const signingIn = login?.state === 'signing_in'
  const loginBusy = login !== undefined && BUSY_STATES.has(login.state)

  async function refresh() {
    const result = await api.profiles()
    setProfiles(result.profiles)
  }

  useEffect(() => {
    let cancelled = false
    api.profiles().then((result) => {
      if (!cancelled) setProfiles(result.profiles)
    }, (err: Error) => {
      if (!cancelled) setError(err.message)
    }).finally(() => { if (!cancelled) setLoading(false) })
    return () => { cancelled = true }
  }, [])

  // Poll only the sign-in panel. Checks run once after Firefox closes, or on request.
  useEffect(() => {
    if (!managedId) return
    let cancelled = false
    let timer: number
    async function poll() {
      try {
        const status = await api.loginStatus(managedId!)
        if (!cancelled) setStatuses((current) => ({ ...current, [managedId!]: status }))
      } catch (err) { if (!cancelled) setError((err as Error).message) }
      if (!cancelled) timer = window.setTimeout(poll, 1000)
    }
    void poll()
    return () => { cancelled = true; window.clearTimeout(timer) }
  }, [managedId])

  // The phone-keyboard QR code is shown on the login page; Firefox then opens straight on Google.
  useEffect(() => {
    if (!managedId) { setQr(null); return }
    let cancelled = false
    api.startPairing().then((result) => { if (!cancelled) setQr(result) }, () => { if (!cancelled) setQr(null) })
    return () => { cancelled = true }
  }, [managedId, signingIn])

  // While a QR code is shown, watch for a phone saying hello (or a replaced link after "No").
  const hasQr = qr !== null
  useEffect(() => {
    if (!hasQr) return
    let cancelled = false
    let timer: number
    async function poll() {
      try {
        const result = await api.pairing()
        if (!cancelled) setQr(result)
      } catch { /* the session ended; the QR effects above clear it */ }
      if (!cancelled) timer = window.setTimeout(poll, 1000)
    }
    timer = window.setTimeout(poll, 1000)
    return () => { cancelled = true; window.clearTimeout(timer) }
  }, [hasQr])

  const pendingCode = qr?.code && !qr.confirmed ? qr.code : null

  useEffect(() => {
    if (active || loading || busy) return
    const target = view.kind === 'add'
      ? panel.current?.querySelector<HTMLInputElement>('input')
      : panel.current?.querySelectorAll<HTMLButtonElement>('[data-nav]:not(:disabled)')[view.kind === 'list' ? highlighted : 0]
    target?.focus()
    // Focus only when a panel opens or an operation completes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [view.kind, active, loading, busy, loginBusy, pendingCode])

  async function perform(action: () => Promise<void>) {
    if (operation.current) return
    operation.current = true
    setBusy(true)
    setError(null)
    try { await action() }
    catch (err) { setError((err as Error).message) }
    finally { operation.current = false; setBusy(false) }
  }

  const choose = (profile: api.Profile) => perform(async () => {
    void api.cancelPairing()
    await api.selectProfile(profile.id)
    api.setProfileScope(profile.id)
    setActive(profile)
  })

  const openSignin = (profile: api.Profile) => perform(async () => {
    setView({ kind: 'manage', profile })
    const status = await api.signin(profile.id)
    setStatuses((current) => ({ ...current, [profile.id]: status }))
  })

  function back() {
    if (busy) return
    setError(null)
    if (!loginBusy) { void api.cancelPairing(); setQr(null) }
    setView({ kind: 'list' })
  }

  useEffect(() => {
    if (active) return
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === 'Escape' || (event.key === 'Backspace' && !(event.target instanceof HTMLInputElement))) {
        event.preventDefault()
        back()
        return
      }
      if (event.target instanceof HTMLInputElement || event.target instanceof HTMLSelectElement) return
      if (busy) return
      const buttons = Array.from(panel.current?.querySelectorAll<HTMLButtonElement>('[data-nav]:not(:disabled)') ?? [])
      const index = buttons.indexOf(document.activeElement as HTMLButtonElement)
      const columns = view.kind === 'list' ? 4 : 1
      const offset = ({ ArrowLeft: -1, ArrowRight: 1, ArrowUp: -columns, ArrowDown: columns } as Record<string, number>)[event.key]
      if (offset !== undefined) {
        event.preventDefault()
        buttons[Math.max(0, Math.min(buttons.length - 1, index + offset))]?.focus()
      } else if (view.kind === 'list') {
        const profile = profiles[highlighted]
        if (profile && profile.id !== 'guest') {
          if (event.key === 's') { event.preventDefault(); void openSignin(profile) }
          if (event.key === 'd') { event.preventDefault(); setView({ kind: 'remove', profile }) }
        }
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  })

  if (active) return <App key={active.id} profile={active} onSwitch={() => { setActive(null); setView({ kind: 'list' }); void refresh().catch((err: Error) => setError(err.message)) }} />

  return (
    <main className="app profiles-page" ref={panel}>
      <header className="top"><div className="brand"><span className="logo">▶</span> CouchTube</div></header>
      {view.kind === 'list' && <>
        <h1 className="page-title">Who’s watching?</h1>
        {loading ? <p className="status">Loading profiles…</p> : <div className="profile-grid">
          {profiles.map((profile, index) => <button type="button" data-nav key={profile.id}
            className="profile-card" disabled={busy} onFocus={() => setHighlighted(index)} onClick={() => void choose(profile)}>
            <span className="profile-avatar" style={{ background: profile.color }}>{profile.name.slice(0, 1).toUpperCase()}</span>
            <strong>{profile.name}</strong>
            <span className="profile-note">{profile.id === 'guest' ? 'Browse signed out'
              : statuses[profile.id]?.state === 'signed_in' ? '✓ Signed in' : 'YouTube profile'}</span>
          </button>)}
          <button type="button" data-nav className="profile-card" disabled={busy}
            onFocus={() => setHighlighted(profiles.length)} onClick={() => { setName(''); setView({ kind: 'add' }) }}>
            <span className="profile-avatar add-avatar">+</span><strong>Add profile</strong><span className="profile-note">Connect your YouTube account</span>
          </button>
        </div>}
        {!loading && profiles.length === 0 && <button data-nav onClick={() => void perform(refresh)}>Retry</button>}
        <footer className="hints"><span><kbd>← ↑ ↓ →</kbd>Move</span><span><kbd>Enter</kbd>Choose</span>
          <span><kbd>S</kbd>Sign in</span><span><kbd>D</kbd>Remove profile</span></footer>
      </>}
      {view.kind === 'add' && <>
        <h1 className="page-title">Add a profile</h1>
        <p className="status">Give it a name. You can then scan a QR code to sign in from your phone before opening its own Firefox window.</p>
        <form className="profile-form" onSubmit={(event) => {
          event.preventDefault()
          void perform(async () => {
            const profile = await api.createProfile(name, color)
            await refresh()
            setView({ kind: 'manage', profile })
          })
        }}>
          <label>Name<input className="search-box" value={name} maxLength={40} required disabled={busy}
            onChange={(event) => setName(event.target.value)} placeholder="Your name" /></label>
          <label>Colour<select value={color} disabled={busy} onChange={(event) => setColor(event.target.value)}>
            {COLORS.map((value, index) => <option value={value} key={value}>{['Blue', 'Pink', 'Green', 'Gold', 'Purple', 'Red'][index]}</option>)}
          </select></label>
          <button type="submit" disabled={busy || !name.trim()}>Create and sign in</button>
          <button type="button" disabled={busy} onClick={back}>Cancel</button>
        </form>
      </>}
      {view.kind === 'manage' && <>
        <h1 className="page-title">Connect {view.profile.name} to YouTube</h1>
        <p className="status" role="status">{login?.message ?? 'Getting sign-in status…'}</p>
        <p className="status">{signingIn
          ? 'Complete sign-in in the Firefox window. It closes by itself once you are signed in, then CouchTube checks the login.'
          : 'Choose Open Firefox sign-in to go straight to Google. It closes by itself once you are signed in, then CouchTube checks the login.'}</p>
        {qr && <div className="pairing">
          <img alt="QR code for the phone sign-in link, which is also written below" width={240} height={240} src={`data:image/svg+xml;utf8,${encodeURIComponent(qr.svg)}`} />
          <div className="pairing-text">
            <p>Scan with your phone (same Wi-Fi) to type your email and password there instead of on the TV keyboard.</p>
            {pendingCode ? <div className="pairing-confirm" role="alert">
              <p>A phone connected. Does it show this code?</p>
              <strong className="pairing-code" aria-label={pendingCode.split('').join(' ')}>{pendingCode.slice(0, 3)} {pendingCode.slice(3)}</strong>
              <div className="pairing-buttons">
                <button data-nav disabled={busy} onClick={() => void perform(async () => {
                  await api.confirmPairing(pendingCode)
                  // The code matches, so open Firefox on Google now; the phone types into it.
                  const status = await api.signin(view.profile.id)
                  setStatuses((current) => ({ ...current, [view.profile.id]: status }))
                  setQr(await api.pairing())
                })}>Yes, that’s my phone</button>
                <button data-nav disabled={busy} onClick={() => void perform(async () => {
                  await api.rejectPairing()
                  setQr(await api.pairing())
                })}>No</button>
              </div>
            </div> : qr.confirmed
              ? <p className="pairing-ok" role="status">✓ Phone connected. Firefox is open on Google; type your email and password on the phone.</p>
              : <p className="pairing-url">{qr.url}</p>}
          </div>
        </div>}
        <div className="profile-actions">
          <button data-nav disabled={busy || loginBusy} onClick={() => void choose(view.profile)}>
            {login?.state === 'signed_in' ? 'Continue to CouchTube' : 'Continue without verified sign-in'}
          </button>
          <button data-nav disabled={busy || loginBusy} onClick={() => void openSignin(view.profile)}>{signingIn ? 'Open sign-in again' : 'Open Firefox sign-in'}</button>
          <button data-nav disabled={busy || loginBusy} onClick={() => void perform(async () => {
            const status = await api.loginStatus(view.profile.id, true)
            setStatuses((current) => ({ ...current, [view.profile.id]: status }))
          })}>Check sign-in</button>
          <button data-nav disabled={busy} onClick={back}>Back to profiles</button>
        </div>
      </>}
      {view.kind === 'remove' && <>
        <h1 className="page-title">Remove {view.profile.name}?</h1>
        <p className="status">This deletes this CouchTube profile and its saved sign-in on this machine.</p>
        <div className="profile-actions">
          <button data-nav disabled={busy} onClick={back}>Keep profile</button>
          <button data-nav disabled={busy} onClick={() => void perform(async () => {
            await api.removeProfile(view.profile.id)
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
