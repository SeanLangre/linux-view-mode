import * as api from './api'
import { formatDuration } from './format'

export const MUSIC_COLOR = '#53a888'

/** The strip along the bottom of every Music screen: what plays, how far, and what comes next. */
export default function MusicBar({ now }: { now: api.MusicState | null }) {
  if (!now || (!now.track && !now.error)) return null
  const track = now.track
  const duration = now.duration ?? track?.duration ?? null
  const fraction = duration && now.position !== null ? Math.min(1, Math.max(0, now.position / duration)) : 0
  return (
    <section className="m-bar" aria-label="Now playing">
      {track && <>
        <img className="m-bar-art" src={track.thumb} alt="" onError={(e) => { e.currentTarget.style.visibility = 'hidden' }} />
        <div className="m-bar-text">
          <strong>{track.title}</strong>
          <span>{[track.artist, now.loaded ? null : track.state === 'downloading' ? `Downloading${track.progress !== null ? ` ${Math.round(track.progress * 100)}%` : '…'}` : 'Loading…'].filter(Boolean).join(' · ')}</span>
        </div>
        <div className="m-bar-progress" aria-hidden="true">
          <span>{now.position !== null ? formatDuration(now.position) : '–:––'}</span>
          <div className="m-bar-line"><div style={{ width: `${fraction * 100}%` }} /></div>
          <span>{duration ? formatDuration(duration) : '–:––'}</span>
        </div>
        <div className="m-bar-state">
          <span title={now.paused ? 'Paused' : 'Playing'}>{now.paused ? '⏸' : '▶'}</span>
          <span className={now.shuffle ? 'on' : 'off'} title="Shuffle">⇄</span>
          <span className={now.repeat === 'off' ? 'off' : 'on'} title={`Repeat ${now.repeat}`}>{now.repeat === 'one' ? '↻1' : '↻'}</span>
          <span title="Volume">{now.muted ? '🔇' : '🔊'} {Math.round(now.volume)}</span>
        </div>
        {now.next && <div className="m-bar-next">Next: {now.next.title}</div>}
      </>}
      {now.error && <div className="m-bar-error" role="alert">{now.error}</div>}
    </section>
  )
}
