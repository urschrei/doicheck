# DOI Checker

A small desktop application that takes a PDF or DOCX, finds its bibliography,
extracts the DOIs, checks them against Crossref, and reports any inconsistencies.
Built with Tauri (Rust backend) and Svelte. Targets macOS and Windows.

## What it does

- Opens a PDF or `.docx` (drag-and-drop or file picker) and fingerprints it
  (SHA-256), so a document seen before shows its stored report immediately.
- Detects the references section and segments it into entries; falls back to a
  whole-document DOI scan when no heading is found.
- Extracts and normalises DOIs, resolves each against Crossref, and fuzzy-matches
  the returned metadata (title, first author, year, container) against the
  reference text, recording mismatches.
- Flags references that have no DOI but a likely Crossref match.
- Caches Crossref responses by DOI in a local SQLite database, so a DOI seen in
  any document is fetched once; the report and progress show how many results
  came from the cache versus a fresh fetch.
- Retries transient Crossref failures with backoff; entries that still could not
  be checked are marked and can be re-checked on their own once you are back
  online, without re-reading the document.
- Lets you mark an individual field mismatch as a false positive (per document);
  dismissals persist across re-checks and are excluded from the issue counts.
- Shows results as per-entry cards (problems first) and exports the report as
  plain text, JSON, or CSV.
- Checks GitHub releases for a newer version on launch and offers to install it.

## Installing

Download the installer for your platform from the
[latest release](https://github.com/urschrei/doicheck/releases/latest).

The app is not signed with a paid developer certificate yet, so the operating
system warns on first launch:

- **macOS:** move `DOI Checker.app` to `/Applications`, then clear the Gatekeeper
  quarantine flag once and open it:

  ```sh
  xattr -dr com.apple.quarantine "/Applications/DOI Checker.app"
  ```

  On macOS Sequoia the Control-click -> Open shortcut no longer bypasses
  Gatekeeper; the command above (or System Settings -> Privacy & Security ->
  "Open Anyway") is the way in.
- **Windows:** SmartScreen may warn — choose "More info" -> "Run anyway".

## Requirements (to build from source)

- [Rust](https://rustup.rs) (stable) and [Node.js](https://nodejs.org) (22+).
- macOS or Windows. Crossref lookups need a network connection.

## Develop

```sh
npm install
npm run tauri dev
```

The Crossref "polite pool" contact email defaults to a built-in value and can be
changed in Settings, along with a default reports folder.

## Test, lint, format

```sh
cd src-tauri
cargo nextest run
cargo clippy --no-deps --all-targets -- -D warnings
cargo fmt --check
```

## Build a release locally

Because updater artifacts are enabled, building a bundle needs the signing key:

```sh
export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.doicheck-updater.key)"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
npm run tauri build
```

`npm run tauri dev` does not bundle and needs none of this. Day-to-day releases
are produced by CI (see Releasing), so local bundle builds are rarely necessary.

## Releasing (maintainers)

Releases are built by GitHub Actions (`.github/workflows/release.yml`) when a tag
matching `v*` is pushed, producing macOS and Windows bundles plus the updater
manifest, attached to a draft GitHub release that you then publish.

One-time setup:

1. Generate an updater signing key (kept out of the repository):

   ```sh
   npm run tauri signer generate -- --ci -w ~/.doicheck-updater.key
   ```

   Put the public key (the `.pub` file's contents) into
   `src-tauri/tauri.conf.json` under `plugins.updater.pubkey`.

2. Add two repository secrets in GitHub:
   - `TAURI_SIGNING_PRIVATE_KEY` — the contents of the private key file.
   - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` — the key's password (empty if none).

To cut a release:

1. Bump the version and commit. Three files carry it and should be kept in sync:
   - `src-tauri/tauri.conf.json` (`version`) — the source of truth. It sets the
     installer/bundle **filenames**, the in-app **About** version
     (`getVersion()`), and the version written into the updater's `latest.json`.
     (`Cargo.toml`'s version is only a fallback used when this key is absent.)
   - `src-tauri/Cargo.toml` (`[package] version`).
   - `package.json` (`version`) — npm metadata only; kept in sync for tidiness.

   The git tag does **not** set the version: if `tauri.conf.json` is left
   unchanged, the build keeps the old number and the updater will not treat the
   release as newer than installed copies.
2. `git push origin main`, then `git tag vX.Y.Z && git push origin vX.Y.Z`.
3. Review and publish the resulting draft release. The auto-updater serves the
   most recently published release.

## Auto-update

On launch the app fetches
`https://github.com/urschrei/doicheck/releases/latest/download/latest.json`,
verifies the update with the bundled public key, and offers to download and
install a newer version. Updates are signed by the updater key above; this is
independent of platform code signing.

## Development

Day-to-day development notes — architecture, the processing pipeline, the data
model, testing, CI, and releasing — are in [`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md).

## License

[Blue Oak Model License 1.0.0](LICENSE.md).
