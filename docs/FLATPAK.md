# Установка Taurigram через Flatpak

На Flathub Taurigram пока нет, поэтому Flatpak собирается у вас локально
из готового `.deb`. Это занимает пару минут, установка — только для вашего пользователя
(без `sudo`).

## 1. Что нужно

- `flatpak` и `flatpak-builder`:

  | Дистрибутив | Команда |
  |---|---|
  | Arch / CachyOS / Manjaro | `sudo pacman -S flatpak flatpak-builder` |
  | Debian / Ubuntu | `sudo apt install flatpak flatpak-builder` |
  | Fedora | `sudo dnf install flatpak flatpak-builder` |

- `git` и `curl`;
- около 2 ГБ места: при первой сборке скачается рантайм GNOME (`org.gnome.Platform` и `org.gnome.Sdk`).

## 2. Сборка и установка

```bash
git clone --depth 1 --filter=blob:none --sparse https://github.com/n1orio/taurigram.git
cd taurigram
git sparse-checkout set packaging/flatpak
cd packaging/flatpak
./build.sh
```

Скачивается только папка `packaging/flatpak` (скрипт и манифест, несколько КБ) —
исходники клиента для этого не нужны: приложение берётся из готового `.deb`.

Скрипт сам:
1. скачает `.deb` из последнего релиза на GitHub;
2. скачает `flathub/shared-modules` (библиотека для иконки в трее);
3. подключит Flathub и поставит нужный рантайм;
4. соберёт и установит `org.taurigram.desktop`.

### Из своей сборки

Если релиза ещё нет или вы что-то меняли в коде, соберите `.deb` сами и передайте путь.
Для этого нужен полный репозиторий (`git clone https://github.com/n1orio/taurigram.git`):

```bash
cd taurigram/client
npm install --allow-git=all
cp .env.example .env        # впишите TELEGRAM_API_ID и TELEGRAM_API_HASH
npx tauri build --bundles deb
cd ../packaging/flatpak
./build.sh ../../client/tauri/target/release/bundle/deb/Taurigram_*_amd64.deb
```

## 3. Запуск

Taurigram появится в меню приложений. Из терминала:

```bash
flatpak run org.taurigram.desktop
```

## Где лежат данные

Темы, плагины, обои и `taurigram-settings.json`:

```
~/.var/app/org.taurigram.desktop/data/org.taurigram.desktop/
```

Свои темы и плагины кладите в `themes/` и `plugins/` внутри этой папки.

## Обновление

Автообновление внутри Flatpak не работает — обновляйте пересборкой:

```bash
cd taurigram && git pull
cd packaging/flatpak && ./build.sh
```

## Удаление

```bash
flatpak uninstall --user org.taurigram.desktop
# вместе с данными:
flatpak uninstall --user --delete-data org.taurigram.desktop
```

## Права приложения

Манифест (`packaging/flatpak/org.taurigram.desktop.yml`) разрешает: сеть, Wayland/X11,
звук, устройства (камера для звонков, GPU), системные уведомления, трей
и папку «Загрузки». Ограничить права можно через Flatseal или
`flatpak override --user org.taurigram.desktop ...`.

## Если что-то не так

- **Падение «Error 71» на NVIDIA + Wayland** — Taurigram сам выставляет
  `__NV_DISABLE_EXPLICIT_SYNC=1`. Если всё равно падает, отключите аппаратный рендерер
  (интерфейс станет медленнее): `flatpak run --env=WEBKIT_DISABLE_DMABUF_RENDERER=1 org.taurigram.desktop`
  или попробуйте X11: `flatpak run --env=GDK_BACKEND=x11 org.taurigram.desktop`.
- **Нет иконки в трее на GNOME** — нужно расширение «AppIndicator and KStatusNotifierItem Support».
- **`build.sh` пишет, что в релизе нет .deb** — соберите `.deb` сами (раздел «Из своей сборки»).
