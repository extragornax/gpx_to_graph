# PWA + Android APK (TWA via Bubblewrap)

The Axum server exposes a Progressive Web App at the site root and an
Android Trusted Web Activity (TWA) APK wrapper. The TWA is built by a
dedicated `apk-builder` docker-compose service and served by the Rust
server via `/download/android.apk` so users can install it with one tap.

## Architecture

```
+---------------------+        +----------------------+
| gpx-blue (Axum)     |        | apk-builder (Node)   |
|  /                  |        |  fetches /manifest   |
|  /manifest.json     | <----- |  generates TWA proj  |
|  /sw.js             |        |  signs APK           |
|  /download/         |        |  writes:             |
|    android.apk      | -----> |    /data/apk/...     |
|  /.well-known/      |        +----------------------+
|    assetlinks.json  |               |
+---------------------+               | shared volume: apk_data
       ^                              |
       |  reads from /data/apk -------+
       |  (read-only mount)
```

Two separate builds:

- **Rust server** — `cargo build --release --bin server --features server`,
  packaged via root `Dockerfile`. Embeds static HTML/manifest/SW/icons.
- **APK** — `docker compose --profile apk run --rm apk-builder`. Produces
  a signed APK and an `assetlinks.json` in the `apk_data` volume.

The keystore lives in the persistent `apk_keystore` volume — never
regenerated, so the SHA-256 fingerprint stays stable across rebuilds.

## What's in the repo

PWA assets:

- `static/manifest.json` — root Web App Manifest (`start_url: /`).
- `static/sw.js` — service worker (network-first, shell cached).
- `static/icon-192.png` / `static/icon-512.png` — placeholder icons
  (copied from `static/trace/`; replace before release).
- `static/.well-known/assetlinks.json` — embedded fallback (placeholder).
  When `apk-builder` has run, the server prefers the live file from the
  `apk_data` volume.

Server routes (`src/bin/server.rs`):

```
/manifest.json                       embedded
/sw.js                               embedded
/icon-192.png                        embedded
/icon-512.png                        embedded
/.well-known/assetlinks.json         disk if present, else embedded
/download/android.apk                disk, 404 if missing
/download/android/status             {"available": bool}
```

The home page has a "Android APK" nav link, hidden by default. A page
load fetches `/download/android/status` and reveals it when an APK
exists.

APK builder:

- `docker/apk-builder/Dockerfile` — Node 20 + JDK 17 + Android SDK +
  Bubblewrap core.
- `docker/apk-builder/build.js` — programmatic build (fetches web
  manifest, generates Android project, signs, extracts SHA-256, writes
  `assetlinks.json`).
- `docker/apk-builder/package.json` — pins `@bubblewrap/core`.

PWA `<link rel="manifest">`, theme-color, apple-touch-icon,
mobile-web-app meta tags, and `serviceWorker.register('/sw.js')` are
injected into the root form page and each feature page (`col`, `trip`,
`strava_stats`, `roulette`, `meteo`, `ravito`, `toolkit`, `auth`). The
`trace/` subapp keeps its own scoped PWA (`/trace/manifest.json`,
`/trace/sw.js`) and is unchanged.

## Required env

Set these in a `.env` file at the repo root (read by `docker compose`):

| Env                       | Required | Default       | Notes                                             |
|---------------------------|----------|---------------|---------------------------------------------------|
| `TWA_DOMAIN`              | yes      | —             | Production HTTPS host, e.g. `gpx.example.com`.    |
| `TWA_PACKAGE_ID`          | yes      | —             | Android package id, e.g. `com.extragornax.gpx`.   |
| `TWA_KEYSTORE_PASSWORD`   | yes      | —             | Keystore password. Save it. Lose it = new APK.    |
| `TWA_KEY_PASSWORD`        | yes      | —             | Private key password.                             |
| `TWA_KEY_ALIAS`           | no       | `android`     | Alias inside the keystore.                        |
| `TWA_APP_NAME`            | no       | `GPX Tools`   | Long name.                                        |
| `TWA_APP_NAME_SHORT`      | no       | `GPX`         | Launcher / short name.                            |
| `TWA_THEME_COLOR`         | no       | `#0e1424`     | Status bar color.                                 |
| `TWA_BG_COLOR`            | no       | `#f2e9d4`     | Splash background.                                |
| `TWA_APP_VERSION`         | no       | `1.0.0`       | `versionName`.                                    |
| `TWA_APP_VERSION_CODE`    | no       | `1`           | `versionCode` (bump on each Play Store upload).   |
| `TWA_KEY_COUNTRY`         | no       | `FR`          | Used for keystore certificate DN.                 |
| `TWA_SCHEME`              | no       | `https`       | Set to `http` only for local dev testing.         |

Example `.env`:

```
TWA_DOMAIN=gpx.example.com
TWA_PACKAGE_ID=com.extragornax.gpx
TWA_KEYSTORE_PASSWORD=change-me-and-store-it-somewhere-safe
TWA_KEY_PASSWORD=change-me-too
TWA_APP_VERSION=1.0.0
TWA_APP_VERSION_CODE=1
```

The Rust server only needs one new env at runtime:

| Env       | Default       | Notes                                            |
|-----------|---------------|--------------------------------------------------|
| `APK_DIR` | `/data/apk`   | Where the server looks for the APK + assetlinks. |

This is set in `docker-compose.yml`; no manual override needed.

## First run

```bash
# 1. Build images
docker compose build
docker compose --profile apk build

# 2. Start the server so the apk-builder can fetch /manifest.json from it
docker compose up -d gpx-blue

# 3. Build the APK (one-shot; first run takes a few minutes — keystore
#    is generated and Gradle warms up)
docker compose --profile apk run --rm apk-builder

# 4. Refresh the home page. The "Android APK" nav link is now visible.
#    Visiting /download/android.apk downloads the signed APK.
```

The first run also writes `assetlinks.json` into the shared `apk_data`
volume with the real SHA-256 fingerprint. The Rust server serves it at
`/.well-known/assetlinks.json` so installed APKs immediately hide their
URL bar.

## Subsequent runs

Bumping the app version:

```bash
TWA_APP_VERSION=1.0.1 TWA_APP_VERSION_CODE=2 \
  docker compose --profile apk run --rm apk-builder
```

The keystore is reused (volume persists), so the SHA-256 stays the
same — installed APKs continue to verify against `assetlinks.json` and
the new APK is upgradable in place.

Site content changes (HTML, manifest, icons) do **not** need an APK
rebuild. The TWA points at your live HTTPS URL; users get updates on
next launch via the service worker.

## Replace placeholder icons

`static/icon-*.png` are reused from the Trace subapp. Before public
release, replace with branded 192×192 and 512×512 PNGs (`purpose: "any
maskable"` — keep important content within the inner 80%). Then
`docker compose build && docker compose up -d gpx-blue` so the new
icons are baked into the server binary and become visible in the PWA
install dialog. Rebuild the APK only if you want a new launcher icon
in the TWA.

## Backups

The `apk_keystore` volume is the single most important artifact. If
you lose it:

- All future APK builds will be signed with a fresh key.
- Existing installed APKs will be unable to verify against the new
  `assetlinks.json`, so the URL bar reappears.
- Play Store uploads under the same package id will be rejected
  because the signing cert no longer matches.

Back it up after the first successful build:

```bash
docker run --rm -v gpx_to_graph_apk_keystore:/k -v $PWD:/out alpine \
  tar -C /k -czf /out/apk_keystore.tar.gz .
```

Store `apk_keystore.tar.gz`, `TWA_KEYSTORE_PASSWORD`, and
`TWA_KEY_PASSWORD` in a password manager.

## Troubleshooting

**The "Android APK" link never appears.** The server's `APK_DIR`
(`/data/apk`) is empty. Run `docker compose --profile apk run --rm
apk-builder` and check its logs for failure.

**`/.well-known/assetlinks.json` returns `REPLACE_ME` placeholders.**
Same cause — the apk-builder hasn't run yet. Once it has, the server
prefers the disk file over the embedded fallback.

**URL bar shows in the installed APK.** Either (a) the SHA-256 in
`assetlinks.json` doesn't match the APK's signing cert, or (b)
`TWA_DOMAIN` doesn't match the host you installed from. Reinstall the
APK after a successful builder run and verify
`curl https://YOUR_DOMAIN/.well-known/assetlinks.json` returns the
real fingerprint.

**Builder fails fetching the manifest.** The builder talks to
`${TWA_SCHEME}://${TWA_DOMAIN}/manifest.json` from inside the docker
network. For local dev, expose the server on a hostname the builder
can reach and set `TWA_SCHEME=http`.
