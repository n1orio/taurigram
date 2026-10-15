// Мост между веб-клиентом и Rust-частью Tauri (аналог Electron preload.cjs).
// Ethernet Client API: window.ethernetDesktop (алиас window.hermesDesktop).

(function () {
  if (window.ethernetDesktop) return;

  function getInternals() {
    return window.__TAURI_INTERNALS__;
  }

  function invoke(cmd, args) {
    return getInternals().invoke(cmd, args || {});
  }

  function listen(event, cb) {
    const internals = getInternals();
    let eventId;
    let isDisposed = false;
    const handler = internals.transformCallback((e) => cb(e.payload));
    invoke('plugin:event|listen', { event, target: { kind: 'Any' }, handler }).then((id) => {
      eventId = id;
      if (isDisposed) unlisten();
    });
    function unlisten() {
      isDisposed = true;
      if (eventId === undefined) return;
      window.__TAURI_EVENT_PLUGIN_INTERNALS__?.unregisterListener(event, eventId);
      invoke('plugin:event|unlisten', { event, eventId });
      eventId = undefined;
    }
    return unlisten;
  }

  // `blob:`-ссылки живут только в этом окне — окну уведомлений и ОС передаём `data:`-URL
  async function toDataUrl(url) {
    if (typeof url !== 'string' || !url.startsWith('blob:')) return url;
    try {
      const blob = await (await fetch(url)).blob();
      return await new Promise((resolve) => {
        const reader = new FileReader();
        reader.onload = () => resolve(reader.result);
        reader.onerror = () => resolve(undefined);
        reader.readAsDataURL(blob);
      });
    } catch {
      return undefined;
    }
  }

  const desktopBridge = {
    // Темы
    themesList: () => invoke('ethernet_themes_list'),
    themeRead: (name) => invoke('ethernet_theme_read', { name }),
    themeSave: (name, css, wallpaperInfo) => invoke('ethernet_theme_save', { name, css, wallpaperInfo }),
    themeDelete: (name) => invoke('ethernet_theme_delete', { name }),
    themeActivate: (name) => invoke('ethernet_theme_activate', { name }),
    // Плагины
    pluginsList: () => invoke('ethernet_plugins_list'),
    pluginRead: (id) => invoke('ethernet_plugin_read', { id }),
    pluginSave: (plugin) => invoke('ethernet_plugin_save', { plugin }),
    pluginDelete: (id) => invoke('ethernet_plugin_delete', { id }),
    pluginToggle: (id) => invoke('ethernet_plugin_toggle', { id }),
    // Служебное
    pickFile: (kind) => invoke('ethernet_pick_file', { kind }),
    saveFile: (payload) => invoke('ethernet_save_file', { payload }),
    reload: () => invoke('ethernet_reload'),
    // Настройки мода (скругления/блюр/анимации)
    modGet: () => invoke('ethernet_mod_get'),
    modSet: (mod) => invoke('ethernet_mod_set', { mod }),
    // Ethernet-обои
    wallpaperSetFile: (payload) => invoke('ethernet_wallpaper_set_file', { payload }),
    wallpaperClear: (themeName) => invoke('ethernet_wallpaper_clear', { themeName }),
    // Управление окном (топбар)
    windowMinimize: () => invoke('ethernet_window_minimize'),
    windowMaximizeToggle: () => invoke('ethernet_window_maximize_toggle'),
    windowClose: () => invoke('ethernet_window_close'),
    windowIsMaximized: () => invoke('ethernet_window_is_maximized'),
    onWindowMaximizedChange: (cb) => listen('hermes:window-maximized-changed', (isMax) => cb(isMax)),
    // Гео-IP
    networkGeoInfo: () => invoke('ethernet_network_geo_info'),
    onNetworkChange: (cb) => listen('hermes:network-changed', () => cb()),
    // Пользовательский прокси (SOCKS5, HTTP)
    proxyGetState: () => invoke('ethernet_proxy_get_state'),
    proxySave: (proxy) => invoke('ethernet_proxy_save', { proxy }),
    proxyDelete: (id) => invoke('ethernet_proxy_delete', { id }),
    proxySetActive: (id) => invoke('ethernet_proxy_set_active', { id }),
    proxyToggle: (enabled) => invoke('ethernet_proxy_toggle', { enabled }),
    proxyParseLink: (link) => invoke('ethernet_proxy_parse_link', { link }),
    proxyTestPing: (proxy) => invoke('ethernet_proxy_test_ping', { proxy }),
    // Кастомные всплывающие уведомления
    showNotification: async (payload) => {
      const icon = await toDataUrl(payload?.icon);
      return invoke('ethernet_show_notification', { payload: { ...payload, icon } });
    },
    onNotificationClick: (cb) => listen('hermes:notification-clicked', (data) => cb(data)),
    // Глобальное отключение уведомлений
    notificationsDisabledGet: () => invoke('ethernet_notifications_disabled_get'),
    notificationsDisabledSet: (disabled) => invoke('ethernet_notifications_disabled_set', { disabled: Boolean(disabled) }),
    onNotificationsDisabledChange: (cb) => listen('hermes:notifications-disabled-changed', (disabled) => cb(disabled)),
    // Не сворачивать в трей (закрывать по крестику)
    closeToTrayDisabledGet: () => invoke('ethernet_close_to_tray_disabled_get'),
    closeToTrayDisabledSet: (disabled) => invoke('ethernet_close_to_tray_disabled_set', { disabled: Boolean(disabled) }),
    // Безопасный режим (Safe Mode) для плагинов
    safeModeGet: () => invoke('ethernet_safemode_get'),
    safeModeSet: (enabled) => invoke('ethernet_safemode_set', { enabled: Boolean(enabled) }),
    lastCrashedPluginGet: () => invoke('ethernet_last_crashed_plugin_get'),
    clearLastCrashedPlugin: () => invoke('ethernet_clear_last_crashed_plugin'),
    reportPluginCrash: (pluginId, error) => invoke('ethernet_report_plugin_crash', {
      pluginId: String(pluginId),
      error: error === undefined ? undefined : String(error),
    }),
    // Экспорт сессии в Telegram Desktop (tdata)
    exportTdata: (session) => invoke('ethernet_export_tdata', { session }),
    // Счётчик непрочитанных для панели задач / трея
    setUnreadCount: (count) => invoke('ethernet_set_unread_count', { count: Number(count) || 0 }),
  };

  Object.freeze(desktopBridge);
  Object.defineProperty(window, 'ethernetDesktop', { value: desktopBridge });
  Object.defineProperty(window, 'hermesDesktop', { value: desktopBridge });

  // Перетаскивание окна за кастомный топбар лоадера (#ethernet-titlebar).
  // Двойной клик разворачивает окно: `mousedown` с `detail === 2` надёжнее `dblclick`,
  // который после начала перетаскивания на Windows не приходит.
  const TITLEBAR_SELECTOR = '#ethernet-titlebar';
  const NO_DRAG_SELECTOR = '.ethernet-titlebar-controls, button, a, input, textarea, select';

  function isDragTarget(target) {
    return target instanceof Element && target.closest(TITLEBAR_SELECTOR) && !target.closest(NO_DRAG_SELECTOR);
  }

  window.addEventListener('mousedown', (e) => {
    if (e.button !== 0 || !isDragTarget(e.target)) return;
    if (e.detail === 2) {
      desktopBridge.windowMaximizeToggle();
    } else {
      invoke('plugin:window|start_dragging');
    }
  }, true);

  window.addEventListener('dblclick', (e) => {
    if (isDragTarget(e.target)) e.stopPropagation();
  }, true);

  // DevTools: F12 / Ctrl+Shift+I
  window.addEventListener('keydown', (e) => {
    const isDevtoolsCombo = e.key === 'F12' || (e.ctrlKey && e.shiftKey && e.key.toLowerCase() === 'i');
    if (!isDevtoolsCombo) return;
    e.preventDefault();
    invoke('ethernet_toggle_devtools');
  }, true);
})();
