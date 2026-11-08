// Driven by twitch_smoke.py --browser. Uses the system Chromium and Node's CDP WebSocket.
import { spawn } from 'node:child_process'
import { mkdir, writeFile } from 'node:fs/promises'
import { setTimeout as delay } from 'node:timers/promises'
import assert from 'node:assert/strict'

const [base, directory] = process.argv.slice(2)
await mkdir(`${directory}/chromium`, { recursive: true })
const chromium = spawn('chromium', ['--headless', '--no-sandbox', '--disable-gpu', '--no-first-run',
  '--disable-dev-shm-usage', '--remote-debugging-port=0', `--user-data-dir=${directory}/chromium`,
  '--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE 127.0.0.1, EXCLUDE localhost', 'about:blank'],
{ stdio: ['ignore', 'ignore', 'pipe'] })
let socket
try {
  const endpoint = await new Promise((resolve, reject) => {
    let output = ''
    const timer = setTimeout(() => reject(new Error('Chromium did not start')), 10000)
    chromium.stderr.on('data', (data) => {
      output += data.toString()
      const match = output.match(/DevTools listening on ws:\/\/(127\.0\.0\.1:\d+)/)
      if (match) { clearTimeout(timer); resolve(`http://${match[1]}`) }
    })
    chromium.once('exit', (code) => { clearTimeout(timer); reject(new Error(`Chromium exited: ${code}`)) })
  })
  const pages = await (await fetch(`${endpoint}/json/list`)).json()
  const page = pages.find((item) => item.type === 'page')
  socket = new WebSocket(page.webSocketDebuggerUrl)
  await new Promise((resolve, reject) => { socket.onopen = resolve; socket.onerror = reject })
  let nextId = 0
  const pending = new Map()
  const errors = []
  socket.onmessage = ({ data }) => {
    const message = JSON.parse(data)
    if (message.method === 'Runtime.exceptionThrown') errors.push(message.params)
    const callbacks = pending.get(message.id)
    if (callbacks) { pending.delete(message.id); message.error ? callbacks.reject(message.error) : callbacks.resolve(message.result) }
  }
  async function send(method, params = {}) {
    const id = ++nextId
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => { pending.delete(id); reject(new Error(`CDP timed out: ${method}`)) }, 5000)
      pending.set(id, { resolve: (value) => { clearTimeout(timer); resolve(value) }, reject: (error) => { clearTimeout(timer); reject(error) } })
      socket.send(JSON.stringify({ id, method, params }))
    })
  }
  async function evaluate(expression) {
    const result = await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true })
    if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails))
    return result.result.value
  }
  async function wait(expression) {
    const deadline = Date.now() + 8000
    while (Date.now() < deadline) {
      if (await evaluate(`Boolean(${expression})`)) return
      await delay(50)
    }
    throw new Error(`UI condition failed: ${expression}\n${await evaluate('document.body.innerText')}`)
  }
  async function key(value) {
    // Let React finish the screen transition and install its keyboard handlers.
    await evaluate('new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))).then(() => true)')
    const code = { Enter: 13, Escape: 27, ArrowDown: 40, ArrowRight: 39, ArrowUp: 38, ArrowLeft: 37, p: 80, d: 68 }[value]
    await send('Input.dispatchKeyEvent', { type: 'keyDown', key: value, windowsVirtualKeyCode: code, text: value === 'Enter' ? String.fromCharCode(13) : undefined })
    await send('Input.dispatchKeyEvent', { type: 'keyUp', key: value, windowsVirtualKeyCode: code })
  }
  await send('Runtime.enable')
  await send('Page.enable')
  await send('Emulation.setDeviceMetricsOverride', { width: 1920, height: 1080, deviceScaleFactor: 1, mobile: false })
  async function shot(name) {
    const { data } = await send('Page.captureScreenshot', { format: 'png' })
    await writeFile(`${directory}/${name}.png`, Buffer.from(data, 'base64'))
  }
  const title = () => evaluate("document.querySelector('.page-title')?.textContent")
  const focused = () => evaluate("document.activeElement?.textContent ?? ''")

  // First screen: YouTube is the default highlight; Twitch is one arrow away.
  await send('Page.navigate', { url: base })
  await wait("document.querySelector('.mode-card') && document.activeElement?.textContent.includes('YouTube')")
  await shot('mode-choice')
  await key('ArrowRight')
  await wait("document.activeElement?.textContent.includes('Twitch')")
  await key('Enter')
  await wait("document.querySelector('.page-title')?.textContent === 'Who’s watching Twitch?'")
  await wait("document.activeElement?.textContent.includes('Guest')")
  await key('Enter')
  await wait("document.querySelector('.profile-label')?.textContent === 'Guest' && document.querySelector('.page-title')?.textContent === 'Twitch'")
  await wait("document.activeElement?.textContent.includes('Top live streams')")

  // Top live streams.
  await key('Enter')
  await wait("document.querySelector('.tw-card .card-title')?.textContent === 'Hello'")
  assert.match(await evaluate("document.querySelector('.tw-card .card-meta').textContent"), /shroud · Game · 4\.2K watching/)
  assert.equal(await evaluate("document.querySelector('.tw-card .badge.live')?.textContent"), 'LIVE')
  await wait("document.activeElement?.classList.contains('tw-card')")

  // Watching. Hold Twitch's player script so the page stays "connecting" and the session stays open.
  const status = async () => (await (await fetch(`${base}/api/status`)).json())
  await send('Fetch.enable', { patterns: [{ urlPattern: 'https://player.twitch.tv/*' }] })
  await key('ArrowRight') // The other card: the focus must come back to the one we leave from.
  await key('ArrowLeft')
  await key('Enter')
  await wait("document.querySelector('iframe.player-frame')?.src.includes('/twitch.html?c=shroud')")
  const playing = await status()
  assert.equal(playing.playing, true)
  assert.equal(playing.id, 'twitch:shroud')
  // Esc reaches the player page (it has the focus), which tells the backend to stop.
  await wait("document.querySelector('iframe.player-frame')?.contentDocument?.readyState === 'interactive'")
  await key('Escape')
  await wait("!document.querySelector('iframe.player-frame')")
  assert.equal((await status()).playing, false)
  await wait("document.activeElement?.classList.contains('tw-card')")
  assert.equal(await evaluate("document.querySelector('.status.error')"), null)

  // A page that cannot load Twitch's player fails with a message, which any key dismisses.
  await send('Fetch.disable')
  await key('Enter')
  await wait("document.querySelector('.status.error')?.textContent.includes('could not play this stream')")
  assert.equal((await status()).error !== null, true)
  await key('ArrowRight')
  await wait("!document.querySelector('.status.error')")
  assert.equal((await status()).error, null)
  await wait("document.activeElement?.classList.contains('tw-card')")
  await key('Escape')
  await wait("document.querySelector('.page-title')?.textContent === 'Twitch'")
  assert.match(await focused(), /Top live streams/)

  // Categories, then one category's streams; going back restores the focused card.
  await key('ArrowRight')
  await key('Enter')
  await wait("document.querySelectorAll('.tw-card').length === 24")
  await shot('categories')
  await key('ArrowRight')
  assert.match(await focused(), /Grand Theft Auto V/)
  await key('Enter')
  await wait("document.querySelector('.page-title')?.textContent === 'Grand Theft Auto V' && document.querySelector('.tw-card .card-title')?.textContent === 'Hello'")
  await key('Escape')
  await wait("document.querySelector('.page-title')?.textContent === 'Categories'")
  await wait("document.activeElement?.textContent.includes('Grand Theft Auto V')")
  await wait("[...document.querySelectorAll('button')].some(b => b.textContent === 'Load more')")
  // Load more shows the next cards and puts the focus on the first new one.
  await evaluate("[...document.querySelectorAll('button')].find(b => b.textContent === 'Load more').focus()")
  await key('Enter')
  await wait("document.querySelectorAll('.tw-card').length === 26")
  await wait("document.activeElement?.textContent.includes('Game 24')")
  await wait("![...document.querySelectorAll('button')].some(b => b.textContent === 'Load more')")
  await key('Escape')
  await wait("document.querySelector('.page-title')?.textContent === 'Twitch'")

  // Search: type, Enter, an offline channel says so.
  assert.match(await focused(), /Categories/) // the focus was remembered
  await key('ArrowRight')
  assert.match(await focused(), /Search/)
  await key('Enter')
  await wait("document.activeElement?.tagName === 'INPUT'")
  await send('Input.insertText', { text: 'retro' })
  await key('Enter')
  await wait("document.querySelector('.tw-card .card-title')?.textContent === 'LIRIK'")
  assert.match(await evaluate("document.querySelector('.tw-card .card-meta').textContent"), /Offline/)
  await key('ArrowDown')
  await wait("document.activeElement?.classList.contains('tw-card')")
  await key('Enter')
  await wait("document.querySelector('.status')?.textContent.includes('LIRIK is offline')")
  await shot('search')
  await key('Escape')
  await wait("document.querySelector('.page-title')?.textContent === 'Twitch'")

  // Esc from Home: profile list; Esc again: the service choice.
  await key('Escape')
  await wait("document.querySelector('.page-title')?.textContent === 'Who’s watching Twitch?'")
  await key('Escape')
  await wait("document.querySelector('.mode-card') && document.activeElement?.textContent.includes('Twitch')")
  assert.deepEqual(errors, [], 'browser raised a JavaScript exception')
  console.log('Twitch browser check passed: service choice, Twitch profile, top live, watching (start, Esc stop, error message), categories and back-focus, search, back to the choice.')
} finally {
  socket?.close()
  chromium.kill('SIGTERM')
  await new Promise((resolve) => { if (chromium.exitCode !== null) resolve(); else chromium.once('exit', resolve) })
}
