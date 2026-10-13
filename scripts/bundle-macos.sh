#!/bin/sh
# Build a release rgrip.app (with icon) and a zip for distribution in dist/.
set -e
cd "$(dirname "$0")/.."

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
# Info.plist versions must be numeric: 0.0.1-alpha -> 0.0.1
short=${version%%-*}
arch=$(uname -m)

cargo build --release -p rgrip

app=dist/rgrip.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/rgrip "$app/Contents/MacOS/rgrip"
cp assets/rgrip.icns "$app/Contents/Resources/rgrip.icns"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>rgrip</string>
  <key>CFBundleDisplayName</key><string>rgrip</string>
  <key>CFBundleIdentifier</key><string>dev.rgrip.rgrip</string>
  <key>CFBundleExecutable</key><string>rgrip</string>
  <key>CFBundleIconFile</key><string>rgrip</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$short</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
</dict>
</plist>
PLIST

# Ad-hoc signature (not notarized): required to run on Apple silicon.
codesign --force --deep --sign - "$app"

zip="dist/rgrip-$version-macos-$arch.zip"
rm -f "$zip"
ditto -c -k --keepParent "$app" "$zip"
echo "$zip"
