#!/bin/bash
# Build a native, explicit-target, or universal macOS app and distribution ZIP.
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ "$(uname -s)" != Darwin ]]; then
    echo "macOS packaging requires a Mac with Xcode Command Line Tools." >&2
    exit 1
fi
requested_target="${1:-native}"
case "$requested_target" in
    native|universal|aarch64-apple-darwin|x86_64-apple-darwin) ;;
    *) echo "Usage: $0 [native|universal|aarch64-apple-darwin|x86_64-apple-darwin]" >&2; exit 2 ;;
esac
if [[ -n "${MACOS_NOTARY_PROFILE:-}" && -z "${MACOS_SIGNING_IDENTITY:-}" ]]; then
    echo "MACOS_NOTARY_PROFILE requires MACOS_SIGNING_IDENTITY (Developer ID Application)." >&2
    exit 2
fi
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-12.0}"
metadata=$(cargo metadata --no-deps --format-version 1)
version=$(python3 -c 'import json,sys; print(json.load(sys.stdin)["packages"][0]["version"])' <<< "$metadata")
build_directory=$(python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])' <<< "$metadata")
work_directory=$(mktemp -d)
trap 'rm -rf "$work_directory"' EXIT
app_name="Process Manager"
app="$work_directory/$app_name.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" dist
if [[ "$requested_target" == universal ]]; then
    cargo build --release --locked --target aarch64-apple-darwin
    cargo build --release --locked --target x86_64-apple-darwin
    lipo -create "$build_directory/aarch64-apple-darwin/release/simple-rust-process-manager" \
        "$build_directory/x86_64-apple-darwin/release/simple-rust-process-manager" \
        -output "$app/Contents/MacOS/simple-rust-process-manager"
    architecture=universal
elif [[ "$requested_target" == native ]]; then
    cargo build --release --locked
    cp "$build_directory/release/simple-rust-process-manager" "$app/Contents/MacOS/"
    architecture=$(uname -m)
else
    cargo build --release --locked --target "$requested_target"
    cp "$build_directory/$requested_target/release/simple-rust-process-manager" "$app/Contents/MacOS/"
    architecture=${requested_target%%-apple-darwin}
fi
iconset="$work_directory/AppIcon.iconset"
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" assets/icon.png --out "$iconset/icon_${size}x${size}.png" >/dev/null
    doubled=$((size * 2))
    sips -z "$doubled" "$doubled" assets/icon.png --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/AppIcon.icns"
python3 - "$app/Contents/Info.plist" "$version" "$MACOSX_DEPLOYMENT_TARGET" <<'PY'
import plistlib, sys
with open(sys.argv[1], "wb") as output:
    plistlib.dump({
        "CFBundleName": "Process Manager",
        "CFBundleDisplayName": "Process Manager",
        "CFBundleIdentifier": "com.enviraldesign.simple-rust-process-manager",
        "CFBundleExecutable": "simple-rust-process-manager",
        "CFBundleIconFile": "AppIcon.icns",
        "CFBundlePackageType": "APPL",
        "CFBundleShortVersionString": sys.argv[2],
        "CFBundleVersion": sys.argv[2],
        "LSMinimumSystemVersion": sys.argv[3],
        "NSHighResolutionCapable": True,
        "NSHumanReadableCopyright": "EnviralDesign",
    }, output)
PY
cp README.md "$app/Contents/Resources/README.md"
if [[ -n "${MACOS_SIGNING_IDENTITY:-}" ]]; then
    codesign --force --options runtime --timestamp --sign "$MACOS_SIGNING_IDENTITY" "$app"
else
    codesign --force --sign - "$app"
fi
codesign --verify --deep --strict "$app"
archive="dist/simple-rust-process-manager-${version}-macos-${architecture}.zip"
if [[ -n "${MACOS_NOTARY_PROFILE:-}" ]]; then
    ditto -c -k --sequesterRsrc --keepParent "$app" "$work_directory/notarize.zip"
    xcrun notarytool submit "$work_directory/notarize.zip" --keychain-profile "$MACOS_NOTARY_PROFILE" --wait
    xcrun stapler staple "$app"
    xcrun stapler validate "$app"
fi
# Replace only this script's generated app, after the new bundle is complete.
destination="dist/$app_name.app"
if [[ -e "$destination" ]]; then
    mv "$destination" "$work_directory/previous.app"
fi
ditto "$app" "$destination"
rm -f "$archive"
ditto -c -k --sequesterRsrc --keepParent "$destination" "$archive"
echo "App: $destination"
echo "Archive: $archive"
if [[ -z "${MACOS_SIGNING_IDENTITY:-}" ]]; then
    echo "Local build uses an ad-hoc signature. Developer ID signing and notarization are needed for public distribution."
fi
