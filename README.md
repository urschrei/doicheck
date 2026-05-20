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

## Requirements

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
cargo clippy --all-targets -- -D warnings
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

To cut a release: bump the version in `src-tauri/tauri.conf.json` (and the
`Cargo.toml`/`package.json` versions), commit, then `git tag vX.Y.Z` and push the
tag. Review and publish the resulting draft release. The auto-updater serves the
most recently published release.

## Auto-update

On launch the app fetches
`https://github.com/urschrei/doicheck/releases/latest/download/latest.json`,
verifies the update with the bundled public key, and offers to download and
install a newer version. Updates are signed by the updater key above; this is
independent of platform code signing.

## Design notes

The design specifications and implementation plans for each iteration are under
`docs/superpowers/`.

## License

[Blue Oak Model License 1.0.0](LICENSE.md).
