# PWA + Android APK (TWA via Bubblewrap)

The Axum server now exposes a Progressive Web App at the site root, plus a
Digital Asset Links file so an Android Trusted Web Activity (TWA) can wrap it
into a native-looking APK without a URL bar.

## What was added

Static files (embedded via `include_str!` / `include_bytes!`):

- `static/manifest.json` — root Web App Manifest (`start_url: /`, scope `/`).
- `static/sw.js` — service worker (network-first, shell cached, ignores `/api`,
  `/auth`, `/webhook`, `/share`, `/generate`, `/merge`, websockets).
- `static/icon-192.png` / `static/icon-512.png` — app icons (placeholders
  copied from `static/trace/`; **replace with branded icons before release**).
- `static/.well-known/assetlinks.json` — Android Asset Links (placeholders;
  filled after Bubblewrap signs the APK).

Routes (in `src/bin/server.rs`):

```
/manifest.json
/sw.js
/icon-192.png
/icon-512.png
/.well-known/assetlinks.json
```

PWA `<link rel="manifest">`, theme-color, apple-touch-icon, mobile-web-app
meta tags, and a `serviceWorker.register('/sw.js')` snippet are injected into
each feature page (`col`, `trip`, `strava_stats`, `roulette`, `meteo`,
`ravito`, `toolkit`, `auth`) and the root form page. The `trace/` subapp keeps
its own scoped PWA (`/trace/manifest.json`, `/trace/sw.js`) and is unchanged.

## Verify the PWA

```bash
cargo run --bin server --features server
# then in a browser:
#   open http://localhost:8080/
#   Lighthouse -> PWA audit should pass installable checks
#   Application -> Manifest, Service Workers tabs should both show entries
```

Production needs HTTPS (Caddy already terminates TLS).

## Build the APK with Bubblewrap

Requires Node 18+ and the Android SDK (Bubblewrap downloads JDK + build-tools
on first run).

```bash
npm i -g @bubblewrap/cli
bubblewrap init --manifest=https://YOUR_DOMAIN/manifest.json
# Answer prompts: package id (e.g. com.extragornax.gpx), app name,
# signing key (let Bubblewrap generate one, save the passphrase),
# display mode = standalone, orientation = any.

bubblewrap build           # produces app-release-signed.apk + app-release-bundle.aab
```

Install on a device:

```bash
adb install app-release-signed.apk
```

## Wire Digital Asset Links (kills the URL bar)

After the first signed build, get the SHA-256 fingerprint:

```bash
bubblewrap fingerprint
# or
keytool -list -v -keystore android.keystore -alias android \
  | grep "SHA256:"
```

Edit `static/.well-known/assetlinks.json` and replace:

- `REPLACE_ME.package` -> the package id used in `bubblewrap init`
- `REPLACE_ME:SHA256:FINGERPRINT` -> the SHA-256 above

Rebuild the server (`include_str!` re-embeds the JSON) and redeploy. Verify
publicly:

```bash
curl https://YOUR_DOMAIN/.well-known/assetlinks.json
```

Then reinstall the APK. The Chrome address bar should disappear; if it
does not, the link verifier failed — recheck the package name and fingerprint.

## Update flow

- Web changes: deploy the site, users get them on next launch (service worker
  network-first).
- Native shell changes (icons, splash, package metadata): rerun
  `bubblewrap build` and reinstall / push to Play Store.

## Replace the placeholder icons

The icons in `static/icon-*.png` are reused from the Trace subapp. Before
public release, replace them with branded 192x192 and 512x512 PNGs
(`purpose: "any maskable"` — include 10% safe-zone padding). Then rebuild
the server (icons are `include_bytes!`).
