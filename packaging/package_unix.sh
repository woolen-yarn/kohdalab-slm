#!/usr/bin/env bash
set -euo pipefail
payload=$(cd "$1" && pwd)
out=$2
version=$3
mkdir -p "$out"
out=$(cd "$out" && pwd)
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
case "$(uname -s):$(uname -m)" in
  Darwin:arm64)
    name="kohdalab-slm-$version-macos-arm64"
    app="$scratch/KohdaLab SLM.app"
    mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
    install -m 755 "$payload/kohdalab-slm" "$app/Contents/MacOS/kohdalab-slm"
    for file in LICENSE README.md THIRD_PARTY_NOTICES.md third-party-licenses bindings; do
      cp -R "$payload/$file" "$app/Contents/Resources/"
    done
    cat >"$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>org.kohdalab.slm</string>
<key>CFBundleName</key><string>KohdaLab SLM</string>
<key>CFBundleExecutable</key><string>kohdalab-slm</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>$version</string>
<key>CFBundleVersion</key><string>$version</string>
</dict></plist>
PLIST
    plutil -lint "$app/Contents/Info.plist"
    codesign --force --sign - "$app"
    codesign --verify --strict "$app"
    ditto -c -k --keepParent "$app" "$out/$name-portable.zip"
    ln -s /Applications "$scratch/Applications"
    hdiutil create -fs HFS+ -format UDZO -volname "KohdaLab SLM" -srcfolder "$scratch" "$out/$name.dmg"
    ;;
  Linux:x86_64)
    name="kohdalab-slm-$version-linux-x64"
    mkdir "$scratch/$name"
    cp -R "$payload/." "$scratch/$name/"
    tar -C "$scratch" -czf "$out/$name-portable.tar.gz" "$name"
    pkg="$scratch/deb"
    mkdir -p "$pkg/DEBIAN" "$pkg/usr/bin" "$pkg/usr/share/doc/kohdalab-slm" "$pkg/usr/share/applications"
    install -m 755 "$payload/kohdalab-slm" "$pkg/usr/bin/kohdalab-slm"
    for file in LICENSE README.md THIRD_PARTY_NOTICES.md third-party-licenses bindings; do
      cp -R "$payload/$file" "$pkg/usr/share/doc/kohdalab-slm/"
    done
    cat >"$pkg/DEBIAN/control" <<CONTROL
Package: kohdalab-slm
Version: $version
Architecture: amd64
Maintainer: KohdaLab
Depends: libc6 (>= 2.35), libgcc-s1, libgl1, libx11-6, libxkbcommon0, libxkbcommon-x11-0, libegl1, libgl1-mesa-dri, libwayland-client0
Recommends: xdg-desktop-portal
Description: KohdaLab SLM pattern editor and simulated output
CONTROL
    cat >"$pkg/usr/share/applications/kohdalab-slm.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=KohdaLab SLM
Exec=kohdalab-slm
Terminal=false
Categories=Science;
DESKTOP
    dpkg-deb --root-owner-group --build "$pkg" "$out/$name.deb"
    ;;
  *) echo "Unsupported package host" >&2; exit 1 ;;
esac
