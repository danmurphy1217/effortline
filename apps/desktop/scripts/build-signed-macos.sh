#!/bin/bash
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "This command requires macOS." >&2
  exit 1
fi

if [[ -z "${APPLE_SIGNING_IDENTITY:-}" ]]; then
  echo "Set APPLE_SIGNING_IDENTITY to an Apple Development signing identity." >&2
  exit 1
fi

signing_identity="$APPLE_SIGNING_IDENTITY"
env -u APPLE_SIGNING_IDENTITY pnpm tauri build --debug --bundles app

app_bundle="../../target/debug/bundle/macos/Effortline.app"
if [[ ! -d "$app_bundle" ]]; then
  echo "Tauri did not create the expected app bundle: $app_bundle" >&2
  exit 1
fi

# Tauri's bundle signing can fail when macOS adds provenance attributes to
# build outputs. Clear attributes from this generated bundle before signing.
find "$app_bundle" -exec xattr -c {} +
codesign --force --deep --options runtime \
  --sign "$signing_identity" \
  --identifier com.danmurphy.effortline \
  --timestamp=none \
  "$app_bundle"
codesign --verify --deep --strict "$app_bundle"
signature_details="$(codesign -dv --verbose=4 "$app_bundle" 2>&1)"
if [[ "$signature_details" != *"Identifier=com.danmurphy.effortline"* \
  || "$signature_details" != *"TeamIdentifier="* \
  || "$signature_details" == *"TeamIdentifier=not set"* \
  || "$signature_details" == *"Signature=adhoc"* ]]; then
  echo "The app bundle does not have Effortline's stable Apple signing identity." >&2
  exit 1
fi

echo "Signed app: $app_bundle"
