# Alternative login: Google's own phone sign-in

Plan for signing in **without sending the password over the network**. Firefox on the TV still
does the sign-in, so it gets the real YouTube web cookies that yt-dlp and mpv need
(`--cookies-from-browser`, `src/ytdlp.rs`, `src/player.rs`). The phone only approves the
sign-in, using Google's own methods.

Related: [`encryption-plan.md`](encryption-plan.md). If this works, encrypted typing becomes the
fallback rather than the main path (see "Effect on the encryption plan").

## Status (2026-10-21): shelved, not the main path
First real test, on the user's own account through CouchTube's sign-in window: after the email,
Google showed **only the password page**. There was no "Try another way", so no passkey and no
phone prompt were offered.

What we learned:
- **It depends on the account, not on CouchTube.** Google lists "Use your passkey" only if the
  account already has a passkey, and "Tap Yes" only if a phone prompt is set up. A new, unknown
  Firefox profile can also make Google show fewer options.
- **CouchTube's Firefox prefs aren't the cause.** `config/firefox/user.js` doesn't touch WebAuthn
  or passkeys.
- **Users must not be asked to change their account.** So passkey and prompt can only ever be a
  bonus for accounts that already offer them. They can't be the recommended path or the fix for
  the password crossing the Wi-Fi.
- **Wording was tried and removed.** Text promising passkeys was added to `src/pairing.html`
  and `frontend/src/ProfileGate.tsx`, then taken out by the user. Don't
  reintroduce it unless the tests below show it works. If it comes back, make it conditional
  ("if Google offers *Try another way*"), never a step.
- **Phone keyboard stays the main path**, so [`encryption-plan.md`](encryption-plan.md) is still
  the fix for the password being sent in plain text. The "neither works" case below applies.

Still untested, and only worth doing on an account that already has a passkey or phone prompt:
tests 1 to 5 below (tracked in `TODO.md`).

## Why not copy cookies from another device
The rejected option was signing in on another browser and copying the cookies to the TV.
- Phone browsers (Chrome, Safari) can't export cookies, so a laptop would be needed.
- A session cookie skips 2-step verification, so a stolen cookie is worse than a stolen password.
  Sending one over the LAN has the same sniffing and active-attacker problems, with more at stake.
- Google rotates YouTube cookies and may end sessions that move to another device or IP. Chrome
  ties Google cookies to the device on some platforms (Device Bound Session Credentials).
- It would need extra plumbing: writing into Firefox's `cookies.sqlite` while Firefox is closed.

## The two methods

### 1. Passkey via QR code (best)
1. The user opens Firefox sign-in on the TV and types the email (on the TV keyboard, or with
   phone pairing, which is only the email now).
2. Google offers **Use your passkey**, or the user picks it under "Try another way".
3. Firefox shows a QR code. The user scans it with the phone's camera.
4. The phone and the TV check over **Bluetooth** that they're close to each other. Then the phone
   unlocks the passkey (fingerprint, face or PIN) and the sign-in finishes in Firefox on the TV.

- **Nothing reusable crosses the Wi-Fi.** The passkey never leaves the phone, and each signature
  only works for that one sign-in.
- **It stops the active attacker too.** The Bluetooth check means a relay from somewhere else
  fails, and passkeys are tied to `google.com`, so a fake page can't use them.
- **Needs:** a Bluetooth adapter on the TV box that Firefox can use; Firefox on Linux supporting
  passkeys from a phone (the "hybrid" transport); a passkey on the Google account (Android phones
  signed in to Google usually have one already); and the phone's Bluetooth turned on.

### 2. Google prompt (simpler, weaker)
1. The user types the email on the TV.
2. Google sends "Is it you trying to sign in?" to the phone (Android with Google Play services, or
   iPhone with the Google or Gmail app). The user taps **Yes**, sometimes after matching a number
   shown on the TV.

- **No password is typed**, as long as Google doesn't ask for it. Whether it does depends on the
  account (2-step settings, "skip password when possible").
- **Needs:** no Bluetooth and no browser support, just a phone signed in to the account.
- **It's weaker than a passkey:** the attacker could start a sign-in at the same moment as the user
  (prompt bombing), and the number match is only partial protection. Fine as a second choice.

## What would change in CouchTube (not applied, see Status)
Only if the tests show passkeys or prompts work, and worded as optional ("if Google offers *Try another
way*"): a short hint in `src/pairing.html` and next to the QR code in `frontend/src/ProfileGate.tsx`
(so users don't confuse CouchTube's QR code with Firefox's passkey QR code), a recommended order in
`plan.md` and `README.md` (passkey, Google prompt, phone keyboard, TV keyboard), and the D-Bus
environment in `deploy/couchtube.service` if Firefox can't reach Bluetooth under the service.
Cookie handling doesn't change: the window still closes once `LOGIN_INFO` is seen three times.

## Effect on the encryption plan
- If passkeys work: the phone keyboard sends only the email, plus a password or 2-step code if
  Google falls back to asking for them. `encryption-plan.md` still applies to those, but it's no
  longer the main way in, so it can wait or be reduced to the minimum (encryption without the
  confirmation code).
- If only the Google prompt works: the password usually isn't sent either, so the same reasoning
  applies, with the password step as a more common fallback.
- If neither works: `encryption-plan.md` stays the main path, unchanged.

## Tests to do first (decide before writing code)
Run these only with an account that already has a passkey or phone prompt. A test account that
shows just the password page (like the user's) can't pass them. In CouchTube the route is: create
a profile, **Open Firefox sign-in**, email, then **Try another way** on the password page.

1. **Passkeys on the dev machine** (Firefox 157, Bluetooth adapter present): start the sign-in
   profile the same way CouchTube does (`firefox --new-instance --no-remote --profile <dir>`), go
   to accounts.google.com, choose **Use your passkey**, and check that:
   - a QR code appears (if it doesn't, Firefox on Linux doesn't support passkeys from a phone yet;
     record the version);
   - the phone connects over Bluetooth and the sign-in finishes;
   - `LOGIN_INFO` appears in the profile's `cookies.sqlite` and CouchTube marks the profile signed in.
2. **The same, but started by the backend service** (systemd user unit), to catch missing access
   to Bluetooth or D-Bus.
3. **Google prompt:** sign in with the email only and check whether Google offers the phone
   prompt without asking for a password, and whether the cookies work afterwards.
4. **On the RN96 once Arch is installed:** `bluetoothctl list` shows an adapter, then repeat test 1.
5. **The daily cookie refresh** (`plan.md`, "Keeping the login alive") still works for a profile
   that signed in with a passkey. It should, because the cookies are the same kind.

Record the results in `TODO.md` under the sign-in follow-ups.
