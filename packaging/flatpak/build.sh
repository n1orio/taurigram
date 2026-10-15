#!/usr/bin/env bash
# Собирает и устанавливает Taurigram во Flatpak (для текущего пользователя).
#   ./build.sh                     — скачать .deb последнего релиза с GitHub
#   ./build.sh path/to/app.deb     — использовать свой .deb (npm run dist:linux)
set -euo pipefail

cd "$(dirname "$0")"
REPO="n1orio/taurigram"

if [[ $# -ge 1 ]]; then
  cp "$1" taurigram.deb
else
  echo "Скачиваю .deb последнего релиза $REPO..."
  url=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
    | grep -o '"browser_download_url": *"[^"]*amd64\.deb"' | head -n1 | cut -d'"' -f4)
  if [[ -z "$url" ]]; then
    echo "В последнем релизе нет .deb — соберите его сами и передайте путь: ./build.sh path/to/app.deb" >&2
    exit 1
  fi
  # Недокачанный файл хранится под именем версии, повторный запуск продолжает загрузку
  part="$(basename "$url").part"
  curl -fL --retry 5 --retry-all-errors -C - "$url" -o "$part"
  mv "$part" taurigram.deb
fi

if [[ ! -d shared-modules ]]; then
  git clone --depth 1 https://github.com/flathub/shared-modules.git
fi

flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
flatpak-builder --user --install --force-clean --install-deps-from=flathub \
  --state-dir=.flatpak-builder build-dir org.taurigram.desktop.yml

echo "Готово. Запуск: flatpak run org.taurigram.desktop"
