// Programmatic TWA build using @bubblewrap/core.
//
// Reads config from env, fetches the web manifest from the live server,
// generates an Android project, builds and signs an APK, and writes the
// matching .well-known/assetlinks.json next to the APK so the Rust server
// can serve both.

const fs = require('fs');
const path = require('path');
const https = require('https');
const http = require('http');

const {
  Config,
  JdkHelper,
  AndroidSdkTools,
  GradleWrapper,
  KeyTool,
  TwaGenerator,
  TwaManifest,
  ConsoleLog,
} = require('@bubblewrap/core');

function required(key) {
  const v = process.env[key];
  if (!v || !v.trim()) {
    console.error(`apk-builder: missing required env ${key}`);
    process.exit(2);
  }
  return v.trim();
}

const TWA_DOMAIN = required('TWA_DOMAIN');
const TWA_PACKAGE_ID = required('TWA_PACKAGE_ID');
const TWA_KEYSTORE_PASSWORD = required('TWA_KEYSTORE_PASSWORD');
const TWA_KEY_PASSWORD = required('TWA_KEY_PASSWORD');

const TWA_APP_NAME = process.env.TWA_APP_NAME || 'GPX Tools';
const TWA_APP_NAME_SHORT = process.env.TWA_APP_NAME_SHORT || 'GPX';
const TWA_KEY_ALIAS = process.env.TWA_KEY_ALIAS || 'android';
const TWA_THEME_COLOR = process.env.TWA_THEME_COLOR || '#0e1424';
const TWA_BG_COLOR = process.env.TWA_BG_COLOR || '#f2e9d4';
const TWA_APP_VERSION = process.env.TWA_APP_VERSION || '1.0.0';
const TWA_APP_VERSION_CODE = parseInt(process.env.TWA_APP_VERSION_CODE || '1', 10);
const TWA_KEY_COUNTRY = process.env.TWA_KEY_COUNTRY || 'FR';
const TWA_SCHEME = (process.env.TWA_SCHEME || 'https').replace(/[^a-z]/g, '');

const OUTPUT_DIR = process.env.OUTPUT_DIR || '/data/apk';
const KEYSTORE_DIR = process.env.KEYSTORE_DIR || '/keystore';
const WORK_DIR = process.env.WORK_DIR || '/work';

function fetchJson(url) {
  return new Promise((resolve, reject) => {
    const lib = url.startsWith('https') ? https : http;
    lib
      .get(url, (res) => {
        if (res.statusCode !== 200) {
          reject(new Error(`GET ${url} -> ${res.statusCode}`));
          return;
        }
        let body = '';
        res.on('data', (c) => (body += c));
        res.on('end', () => {
          try {
            resolve(JSON.parse(body));
          } catch (e) {
            reject(e);
          }
        });
      })
      .on('error', reject);
  });
}

async function main() {
  fs.mkdirSync(OUTPUT_DIR, { recursive: true });
  fs.mkdirSync(KEYSTORE_DIR, { recursive: true });
  fs.mkdirSync(WORK_DIR, { recursive: true });

  const log = new ConsoleLog('apk');
  const config = new Config(process.env.JAVA_HOME, process.env.ANDROID_HOME);
  const jdkHelper = new JdkHelper(process, config);
  const sdkTools = await AndroidSdkTools.create(process, config, jdkHelper, log);
  const gradle = new GradleWrapper(process, sdkTools);
  const keyTool = new KeyTool(jdkHelper, log);

  const manifestUrl = `${TWA_SCHEME}://${TWA_DOMAIN}/manifest.json`;
  log.info(`fetching ${manifestUrl}`);
  const webManifest = await fetchJson(manifestUrl);

  const startUrl = new URL(
    webManifest.start_url || '/',
    `${TWA_SCHEME}://${TWA_DOMAIN}/`,
  );
  const twaManifest = await TwaManifest.fromWebManifestJson(startUrl, webManifest);

  twaManifest.packageId = TWA_PACKAGE_ID;
  twaManifest.host = TWA_DOMAIN;
  twaManifest.name = TWA_APP_NAME;
  twaManifest.launcherName = TWA_APP_NAME_SHORT;
  twaManifest.themeColor = TWA_THEME_COLOR;
  twaManifest.navigationColor = TWA_THEME_COLOR;
  twaManifest.backgroundColor = TWA_BG_COLOR;
  twaManifest.appVersionName = TWA_APP_VERSION;
  twaManifest.appVersion = TWA_APP_VERSION;
  twaManifest.appVersionCode = TWA_APP_VERSION_CODE;
  twaManifest.fallbackType = 'customtabs';
  twaManifest.signingKey = {
    path: path.join(KEYSTORE_DIR, 'android.keystore'),
    alias: TWA_KEY_ALIAS,
  };

  const manifestPath = path.join(WORK_DIR, 'twa-manifest.json');
  await twaManifest.saveToFile(manifestPath);

  if (!fs.existsSync(twaManifest.signingKey.path)) {
    log.info('creating signing key (first run; subsequent runs reuse it)');
    await keyTool.createSigningKey(
      {
        path: twaManifest.signingKey.path,
        password: TWA_KEYSTORE_PASSWORD,
        keypassword: TWA_KEY_PASSWORD,
        alias: TWA_KEY_ALIAS,
        fullName: TWA_APP_NAME,
        organizationalUnit: 'engineering',
        organization: TWA_APP_NAME,
        country: TWA_KEY_COUNTRY,
      },
      false,
    );
  }

  const generator = new TwaGenerator();
  await generator.removeTwaProject(WORK_DIR).catch(() => {});
  await generator.createTwaProject(WORK_DIR, twaManifest, log);

  log.info('gradle assembleRelease');
  await gradle.assembleRelease();

  const apkUnsigned = path.join(
    WORK_DIR,
    'app/build/outputs/apk/release/app-release-unsigned.apk',
  );
  const apkSigned = path.join(OUTPUT_DIR, 'app-release-signed.apk');

  log.info('signing APK');
  await sdkTools.apksigner(
    twaManifest.signingKey.path,
    TWA_KEYSTORE_PASSWORD,
    TWA_KEY_ALIAS,
    TWA_KEY_PASSWORD,
    apkUnsigned,
    apkSigned,
  );

  log.info('extracting SHA-256 fingerprint');
  const info = await keyTool.keyInfo({
    path: twaManifest.signingKey.path,
    alias: TWA_KEY_ALIAS,
    keypassword: TWA_KEY_PASSWORD,
    password: TWA_KEYSTORE_PASSWORD,
  });
  const fp = info.fingerprints;
  const sha256 =
    (fp && typeof fp.get === 'function' && fp.get('SHA256')) ||
    (fp && fp.SHA256);
  if (!sha256) {
    throw new Error('SHA-256 fingerprint not available from KeyTool');
  }

  const assetlinks = [
    {
      relation: ['delegate_permission/common.handle_all_urls'],
      target: {
        namespace: 'android_app',
        package_name: TWA_PACKAGE_ID,
        sha256_cert_fingerprints: [sha256],
      },
    },
  ];
  const assetlinksPath = path.join(OUTPUT_DIR, 'assetlinks.json');
  fs.writeFileSync(assetlinksPath, JSON.stringify(assetlinks, null, 2) + '\n');

  log.info(`APK:        ${apkSigned}`);
  log.info(`assetlinks: ${assetlinksPath}`);
  log.info(`SHA-256:    ${sha256}`);
}

main().catch((err) => {
  console.error(err.stack || err);
  process.exit(1);
});
