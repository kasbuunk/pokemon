# Distribution

How Pokémon SRM Editor reaches users, and what each channel costs.

## Channels

| Channel | Cost | Status |
|---|---|---|
| Web app (GitHub Pages) | free | live — deployed from `main` by `.github/workflows/pages.yml` |
| GitHub Releases (macOS/Windows/Linux) | free | tag-triggered by `.github/workflows/release.yml` |
| Apple notarization (Developer ID) | $99/year | documented below, not set up |
| Homebrew cask ([kasbuunk/homebrew-tap](https://github.com/kasbuunk/homebrew-tap)) | free | live — manual version bumps, see below |

## Cutting a release

Either push a tag:

```sh
git tag v0.1.0 && git push origin v0.1.0
```

…or bump `.release-version` on `main` (release-by-commit, for
environments that can push branches but not tags — e.g. Claude
sessions): change the file to the new tag and merge; `cut-release.yml`
builds and publishes the release, creating the tag at that commit. A
`workflow_dispatch` of `release.yml` with the `tag` input does the same
from the Actions UI.

The release workflow builds:

- **macOS**: a universal (arm64 + x86_64) `Pokémon SRM Editor.app` via
  cargo-bundle + `lipo`, **ad-hoc signed**, zipped with `ditto` (which
  preserves bundle metadata) as `Pokemon-SRM-Editor-macos-universal.zip`.
- **Linux**: `Pokemon-SRM-Editor-linux-x86_64.tar.gz` (plain binary).
- **Windows**: `Pokemon-SRM-Editor-windows-x86_64.zip`.
- `SHA256SUMS.txt` over all assets, attached to the GitHub Release together
  with install instructions.

## Why macOS shows a warning, and the free mitigations

The app is ad-hoc signed (`codesign -s -`), not notarized: notarization
requires a paid Apple Developer account. Gatekeeper therefore blocks the
first launch of a downloaded copy until the user right-clicks → Open (or runs
`xattr -dc` on the app). What builds trust without paying:

- the code is open source and the release is built by a public GitHub Actions
  run traceable to a tag;
- `SHA256SUMS.txt` lets users verify the download matches the CI artifact;
- building from source is one command:
  `cargo install --git https://github.com/kasbuunk/pokemon pksave-app`
  (or clone + `cargo run --release -p pksave-app`);
- the ad-hoc signature keeps the bundle identity
  (`com.kasbuunk.pksave`) stable, so macOS TCC grants (the removable-volumes
  prompt for SD-card scanning) persist across launches.

## The paid path (when/if a Developer ID exists)

1. Enroll in the Apple Developer Program ($99/year); create a
   **Developer ID Application** certificate and export it as a `.p12`.
2. Add repo secrets: `MACOS_CERT_P12` (base64), `MACOS_CERT_PASSWORD`,
   `APPLE_ID`, `APPLE_TEAM_ID`, `APPLE_APP_PASSWORD` (app-specific password).
3. In `release.yml`, replace the ad-hoc `codesign -s -` with:

   ```sh
   codesign --force --deep --options runtime \
     -s "Developer ID Application: <name> (<team>)" "$APP"
   ditto -c -k --keepParent "$APP" app.zip
   xcrun notarytool submit app.zip --apple-id "$APPLE_ID" \
     --team-id "$APPLE_TEAM_ID" --password "$APPLE_APP_PASSWORD" --wait
   xcrun stapler staple "$APP"
   ```

4. Gatekeeper then opens the app first try, no right-click dance.

This is the only distribution step that costs money; everything else here is
free.

## Homebrew cask

The personal tap [kasbuunk/homebrew-tap](https://github.com/kasbuunk/homebrew-tap)
holds `Casks/pokemon-srm-editor.rb`, pointing at the release zip. Users
install with
`brew install --cask --no-quarantine kasbuunk/tap/pokemon-srm-editor`
(`--no-quarantine` skips Gatekeeper for the unsigned app).

After each release, bump the cask: update `version` and `sha256` (the
macOS zip's line in the release's `SHA256SUMS.txt`) in the tap repo. The
cask's `livecheck` follows the latest GitHub release, so
`brew livecheck pokemon-srm-editor` reports when the cask lags. Automate
later: a release-workflow step that opens a PR against the tap with the
new version + hash (issue #20 tracks this as an optional follow-up).

## Manual, outside-the-repo checklist (one-time)

1. **Repo settings**: description
   *"Pokémon SRM Editor — free web + desktop save editor for Pokémon
   Red/Blue/Yellow (.srm/.sav, Gen 1)"*; website
   `https://kasbuunk.github.io/pokemon/`.
2. **Topics**: `pokemon`, `save-editor`, `srm`, `sav`, `gen1`, `pokemon-red`,
   `pokemon-blue`, `pokemon-yellow`, `wasm`, `rust`, `egui`, `miyoo-mini`,
   `game-boy`.
3. **Google Search Console**: verify the URL-prefix property
   `https://kasbuunk.github.io/pokemon/` and submit `sitemap.xml` (a
   robots.txt under the subpath is not honored — Search Console is the real
   channel).
4. Push the first tag (`v0.1.0`) once the release workflow is on `main`.
5. Optional: custom domain (makes robots.txt effective and the URL
   memorable). The `kasbuunk/homebrew-tap` repo for the cask exists.
