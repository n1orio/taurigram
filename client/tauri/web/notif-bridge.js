// Мост окна всплывающих уведомлений (аналог Electron preload-notif.cjs)

(function () {
  if (window.notifBridge) return;

  function invoke(cmd, args) {
    return window.__TAURI_INTERNALS__.invoke(cmd, args || {});
  }

  window.notifBridge = {
    onAddNotification(callback) {
      const handler = window.__TAURI_INTERNALS__.transformCallback((e) => callback(e.payload));
      invoke('plugin:event|listen', { event: 'notif:add', target: { kind: 'Any' }, handler })
        .then(() => invoke('notif_ready'));
    },
    clickNotification(data) {
      invoke('notif_clicked', { data });
    },
    dismissNotification() {
      invoke('notif_dismissed');
    },
    // Окно подгоняется по высоте под карточки, поэтому сквозные клики не нужны
    setIgnoreMouse() {},
    setHeight(height) {
      invoke('notif_set_height', { height: Number(height) || 0 });
    },
  };
})();
