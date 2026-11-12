import { useEffect, useRef, useState } from 'react'
import * as api from './api'

/** The phone remote screen: turn it on, pair a phone, confirm its code (the same as on the other services). */
export default function MusicRemote({ onNote }: { onNote: (text: string) => void }) {
  const [remote, setRemote] = useState<api.RemoteInfo | null>(null)
  const section = useRef<HTMLDivElement>(null)

  useEffect(() => {
    let cancelled = false
    let timer: number
    async function poll() {
      try { const info = await api.remoteInfo(); if (!cancelled) setRemote(info) } catch { /* keep the last state */ }
      if (!cancelled) timer = window.setTimeout(poll, 1000)
    }
    void poll()
    return () => { cancelled = true; window.clearTimeout(timer) }
  }, [])

  const items = !remote?.enabled ? [{ label: 'Turn on', action: 'on' }]
    : remote.code && !remote.confirmed ? [{ label: 'Yes, that’s my phone', action: 'yes' }, { label: 'No', action: 'no' }]
    : [{ label: remote.url ? 'New link' : 'Pair a phone', action: 'pair' },
      ...(remote.devices > 0 ? [{ label: `Forget ${remote.devices} phone${remote.devices === 1 ? '' : 's'}`, action: 'forget' }] : []),
      { label: 'Turn off', action: 'off' }]

  async function act(action: string) {
    try {
      if (action === 'on') setRemote(await api.setRemote(true))
      else if (action === 'off') setRemote(await api.setRemote(false))
      else if (action === 'pair') setRemote(await api.remotePair())
      else if (action === 'forget') { await api.forgetPhones(); setRemote(await api.remoteInfo()) }
      else if (action === 'yes' && remote?.code) { await api.remoteConfirm(remote.code); setRemote(await api.remoteInfo()) }
      else if (action === 'no') { await api.remoteReject(); setRemote(await api.remotePair()) }
    } catch (err) { onNote((err as Error).message) }
  }

  // The buttons change with the state: keep the focus on one of them.
  const labels = remote ? items.map((i) => i.label).join('|') : ''
  useEffect(() => {
    if (!labels) return
    const root = section.current?.closest('main')
    if (!root?.contains(document.activeElement) || document.activeElement === root) section.current?.querySelector<HTMLButtonElement>('[data-nav]')?.focus()
  }, [labels])

  if (!remote) return <p className="status">Loading…</p>
  return (
    <div ref={section}>
      <div className="profile-grid">
        {items.map((item) => <button type="button" data-nav key={item.action} className="profile-card" onClick={() => void act(item.action)}>
          <strong>{item.label}</strong></button>)}
      </div>
      <section className="pairing">
        {remote.svg && <img alt="QR code for pairing a phone with the remote" src={`data:image/svg+xml;utf8,${encodeURIComponent(remote.svg)}`} />}
        <div className="pairing-text">
          {!remote.enabled ? <p>The remote is off. Turn it on to control the music from a phone on this network.</p>
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
    </div>
  )
}
