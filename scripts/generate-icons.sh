#!/usr/bin/env bash
# Генерирует все иконки Taurigram из docs/brand/taurigram-icon.svg (цветная) и
# docs/brand/taurigram-glyph.svg (одноцветный знак). Нужны rsvg-convert, ImageMagick и npm-зависимости client/.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BRAND="$ROOT/docs/brand"
ICON="$BRAND/taurigram-icon.svg"
GLYPH="$BRAND/taurigram-glyph.svg"
PUBLIC="$ROOT/client/public"
TAURI_ICONS="$ROOT/client/tauri/icons"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# Квадратный вариант без скругления — для Apple и маскируемых иконок
sed 's#<rect x="32" y="32" width="960" height="960" rx="224"#<rect x="0" y="0" width="1024" height="1024" rx="0"#g' "$ICON" > "$TMP/square.svg"

# Бренд-ресурсы
rsvg-convert -w 1024 -h 1024 "$ICON" -o "$BRAND/taurigram-icon.png"
for s in 16 32 48 256; do rsvg-convert -w $s -h $s "$ICON" -o "$TMP/ico-$s.png"; done
magick "$TMP"/ico-16.png "$TMP"/ico-32.png "$TMP"/ico-48.png "$TMP"/ico-256.png "$BRAND/taurigram.ico"

# Иконки приложения (Windows / Linux / macOS)
(cd "$ROOT/client" && npx tauri icon "$BRAND/taurigram-icon.png" -o "$TAURI_ICONS" > /dev/null)
rm -rf "$TAURI_ICONS"/android "$TAURI_ICONS"/ios "$TAURI_ICONS"/Square*.png "$TAURI_ICONS"/StoreLogo.png
mkdir -p "$TAURI_ICONS/macos"
cp "$TAURI_ICONS"/{32x32.png,64x64.png,128x128.png,128x128@2x.png,icon.icns,icon.ico,icon.png} "$TAURI_ICONS/macos/"
# Шаблонная (чёрная) иконка трея для macOS
sed 's#currentColor#\#000000#g' "$GLYPH" > "$TMP/glyph-black.svg"
rsvg-convert -w 44 -h 44 --keep-aspect-ratio "$TMP/glyph-black.svg" -o "$TAURI_ICONS/tray-macos.png"

# Веб-иконки клиента
cp "$ICON" "$ROOT/client/src/assets/taurigram-logo.svg"
cp "$ICON" "$PUBLIC/favicon.svg"
python3 - "$ICON" "$PUBLIC/favicon-unread.svg" <<'PY'
import sys
s = open(sys.argv[1]).read()
dot = '  <circle cx="820" cy="204" r="150" fill="#F23C34" stroke="#FFFFFF" stroke-width="40"/>\n</svg>'
open(sys.argv[2], 'w').write(s.replace('</svg>', dot))
PY
rsvg-convert -w 16 -h 16 "$ICON" -o "$PUBLIC/favicon-16x16.png"
rsvg-convert -w 32 -h 32 "$ICON" -o "$PUBLIC/favicon-32x32.png"
magick "$TMP"/ico-16.png "$TMP"/ico-32.png "$TMP"/ico-48.png "$PUBLIC/favicon.ico"
cp "$BRAND/taurigram.ico" "$PUBLIC/ethernet.ico"
for s in 192 384 512; do
  rsvg-convert -w $s -h $s "$ICON" -o "$PUBLIC/icon-${s}x${s}.png"
  cp "$PUBLIC/icon-${s}x${s}.png" "$PUBLIC/icon-dev-${s}x${s}.png"
  rsvg-convert -w $s -h $s "$TMP/square.svg" -o "$PUBLIC/icon-square-${s}x${s}.png"
  cp "$PUBLIC/icon-square-${s}x${s}.png" "$PUBLIC/icon-square-dev-${s}x${s}.png"
done
rsvg-convert -w 180 -h 180 "$TMP/square.svg" -o "$PUBLIC/apple-touch-icon.png"
cp "$PUBLIC/apple-touch-icon.png" "$PUBLIC/apple-touch-icon-dev.png"
rsvg-convert -w 512 -h 512 "$ICON" -o "$PUBLIC/icon-electron-macos.png"
rsvg-convert -w 150 -h 150 "$ICON" -o "$PUBLIC/mstile-150x150.png"

# Анимированная заставка при запуске
python3 "$ROOT/scripts/generate-splash.py" "$GLYPH" "$ROOT/client/src/assets/tgs/loading.tgs"

echo "Иконки обновлены. Знак в топбаре окна — client/tauri/web/loader.js (.ethernet-titlebar-logo), берётся из $GLYPH вручную."
