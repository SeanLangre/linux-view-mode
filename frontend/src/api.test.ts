import { afterEach, describe, expect, it, vi } from 'vitest'
import { play, playerStatus, stop } from './api'

afterEach(() => vi.unstubAllGlobals())

describe('playback API', () => {
  it('posts the video and resume position, and posts stop', async () => {
    const fetcher = vi.fn().mockResolvedValue({ ok: true, json: async () => ({}) })
    vi.stubGlobal('fetch', fetcher)
    await play('aqz-KE-bpKQ', 12.5)
    await stop()
    expect(fetcher.mock.calls).toEqual([
      ['/api/play', { method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ id: 'aqz-KE-bpKQ', start: 12.5 }) }],
      ['/api/stop', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{}' }],
    ])
  })

  it('always fetches fresh status and reports API errors', async () => {
    const fetcher = vi.fn()
      .mockResolvedValueOnce({ ok: true, json: async () => ({ playing: true }) })
      .mockResolvedValueOnce({ ok: true, json: async () => ({ playing: false }) })
      .mockResolvedValueOnce({ ok: false, json: async () => ({ error: 'mpv unavailable' }) })
    vi.stubGlobal('fetch', fetcher)
    expect(await playerStatus()).toEqual({ playing: true })
    expect(await playerStatus()).toEqual({ playing: false })
    expect(fetcher.mock.calls[0]).toEqual(['/api/status', { cache: 'no-store' }])
    await expect(play('aqz-KE-bpKQ')).rejects.toThrow('mpv unavailable')
  })
})
