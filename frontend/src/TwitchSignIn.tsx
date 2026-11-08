import { useEffect, useRef, useState } from 'react'
import * as api from './api'

const BUSY = new Set(['signing_in', 'checking'])

/**
 * Signing in to Twitch for one Twitch profile, the way YouTube's works: a QR code opens the phone
 * keyboard, and Firefox opens on Twitch's login page in the profile's own folder. The window closes
 * by itself once Twitch's login cookie appears; the backend then asks Twitch who signed in.
 */
export default function TwitchSignIn({ profile, onContinue, onBack }: {
  profile: api.TwitchProfile
  onContinue: () => void
  onBack: () => void
}) {
  const [login, setLogin] = useState<api.LoginStatus | null>(null)
  const [qr, setQr] = useState<api.Pairing | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const operation = useRef(false)
  const signingIn = login?.state === 'signing_in'
  const loginBusy = login !== null && BUSY.has(login.state)
  const pendingCode = qr?.code && !qr.confirmed ? qr.code : null

  // The sign-in state, once a second.
  useEffect(() => {
    let cancelled = false
    let timer: number
    async function poll() {
      try {
        const status = await api.twitchLoginStatus(profile.id)
        if (!cancelled) setLogin(status)
      } catch (err) { if (!cancelled) setError((err as Error).message) }
      if (!cancelled) timer = window.setTimeout(poll, 1000)
    }
    void poll()
    return () => { cancelled = true; window.clearTimeout(timer) }
  }, [profile.id])

  // The phone link is made before Firefox opens, so the QR code can be scanned first. Leaving the
  // panel cancels it (a link whose window is already open is kept until the window closes).
  useEffect(() => {
    let cancelled = false
    api.startTwitchPairing().then((result) => { if (!cancelled) setQr(result) }, () => { if (!cancelled) setQr(null) })
    return () => { cancelled = true }
  }, [signingIn])
  useEffect(() => () => { void api.cancelPairing() }, [])

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
      } catch { /* the session ended */ }
      if (!cancelled) timer = window.setTimeout(poll, 1000)
    }
    timer = window.setTimeout(poll, 1000)
    return () => { cancelled = true; window.clearTimeout(timer) }
  }, [hasQr])

  async function perform(action: () => Promise<void>) {
    if (operation.current) return
    operation.current = true
    setBusy(true)
    setError(null)
    try { await action() }
    catch (err) { setError((err as Error).message) }
    finally { operation.current = false; setBusy(false) }
  }

  const openSignin = () => perform(async () => { setLogin(await api.twitchSignin(profile.id)) })

  return <>
    <h1 className="page-title">Connect {profile.name} to Twitch</h1>
    <p className="status" role="status">{login?.message ?? 'Getting sign-in status…'}</p>
    <p className="status">Signing in is optional: it lets <strong>Following</strong> show the channels you follow. {signingIn
      ? 'Complete the login in the Firefox window. It closes by itself once you are signed in.'
      : 'Choose Open Firefox sign-in to go to Twitch’s login page. It closes by itself once you are signed in.'}</p>
    {qr && <div className="pairing">
      <img alt="QR code for the phone sign-in link, which is also written below" width={240} height={240} src={`data:image/svg+xml;utf8,${encodeURIComponent(qr.svg)}`} />
      <div className="pairing-text">
        <p>Scan with your phone (same Wi-Fi) to type your Twitch username and password there instead of on the TV keyboard.</p>
        {pendingCode ? <div className="pairing-confirm" role="alert">
          <p>A phone connected. Does it show this code?</p>
          <strong className="pairing-code" aria-label={pendingCode.split('').join(' ')}>{pendingCode.slice(0, 3)} {pendingCode.slice(3)}</strong>
          <div className="pairing-buttons">
            <button data-nav disabled={busy} onClick={() => void perform(async () => {
              await api.confirmPairing(pendingCode)
              // The code matches: open Twitch's login page now; the phone types into it.
              setLogin(await api.twitchSignin(profile.id))
              setQr(await api.pairing())
            })}>Yes, that’s my phone</button>
            <button data-nav disabled={busy} onClick={() => void perform(async () => {
              await api.rejectPairing()
              setQr(await api.pairing())
            })}>No</button>
          </div>
        </div> : qr.confirmed
          ? <p className="pairing-ok" role="status">✓ Phone connected. Firefox is open on Twitch; type your username on the phone, then your password.</p>
          : <p className="pairing-url">{qr.url}</p>}
      </div>
    </div>}
    <div className="profile-actions">
      <button data-nav disabled={busy || loginBusy} onClick={onContinue}>
        {login?.state === 'signed_in' ? 'Continue to Twitch' : 'Continue without signing in'}
      </button>
      <button data-nav disabled={busy || loginBusy} onClick={() => void openSignin()}>{signingIn ? 'Open sign-in again' : 'Open Firefox sign-in'}</button>
      <button data-nav disabled={busy || loginBusy} onClick={() => void perform(async () => { setLogin(await api.twitchLoginStatus(profile.id, true)) })}>Check sign-in</button>
      <button data-nav disabled={busy} onClick={onBack}>Back to profiles</button>
    </div>
    {busy && <p className="status" role="status">Please wait…</p>}
    {error && <p className="status error" role="alert">{error}</p>}
  </>
}
