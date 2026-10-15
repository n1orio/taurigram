/**
 * Plugin: Ghost Mode (в духе AyuGram)
 *
 * - не отправляет отметки о прочтении сообщений и историй;
 * - не отправляет статус «в сети» и после любых действий снова уходит в «не в сети»;
 * - не отправляет «печатает…» / «записывает…»;
 * - по желанию: читает чат при отправке в него сообщения;
 * - кнопка-призрак в шапке списка чатов: клик — вкл/выкл, правый клик — настройки;
 * - Ctrl+Shift+G — вкл/выкл, Alt+R — прочитать открытый чат вручную;
 * - при выключении открытый чат сразу отмечается прочитанным.
 *
 * Работает поверх `window.ethernet.api.beforeRequest`, настройки — в localStorage.
 * Главный переключатель общий со встроенным режимом призрака клиента (`window.taurigramGhost`):
 * 👻 включает и выключает оба сразу.
 */

(function () {
  const PLUGIN_ID = 'ghost-mode';
  const STORAGE_KEY = 'taurigram-ghost-plugin';
  const STYLE_ID = 'taurigram-ghost-plugin-styles';
  const BUTTON_ID = 'taurigram-ghost-button';
  const PANEL_ID = 'taurigram-ghost-panel';
  const OFFLINE_DELAY_MS = 1500;

  const OPTIONS = [
    ['dontReadMessages', 'Не читать сообщения', 'Собеседник не увидит двойную галочку'],
    ['dontReadStories', 'Не читать истории', 'Вы не появитесь в списке просмотревших'],
    ['dontSendOnline', 'Не отправлять «в сети»', 'Клиент не сообщает серверу, что вы онлайн'],
    ['sendOfflineAfterOnline', 'Уходить в «не в сети» после действий', 'Сразу после отправки, реакции и т.п.'],
    ['dontSendTyping', 'Не отправлять «печатает…»', 'Также «записывает голосовое», «отправляет фото»'],
    ['readOnSend', 'Читать чат при отправке сообщения', 'Как «Mark read after action» в AyuGram'],
  ];

  const DEFAULTS = {
    enabled: true,
    dontReadMessages: true,
    dontReadStories: true,
    dontSendOnline: true,
    sendOfflineAfterOnline: true,
    dontSendTyping: true,
    readOnSend: false,
  };

  // Методы `callApi`, после которых сервер считает пользователя «в сети»
  const ACTIVITY_METHODS = new Set([
    'sendMessage', 'editMessage', 'forwardMessages', 'sendReaction', 'sendPaidReaction',
    'sendStoryReaction', 'sendPollVote', 'pinMessage', 'deleteMessages',
  ]);

  const ethernet = window.ethernet || window.hermes;
  if (!ethernet?.api?.beforeRequest) {
    console.warn(`[${PLUGIN_ID}] window.ethernet.api недоступен — плагин не запущен`);
    return;
  }
  const { api, store, log, addStyle, on } = ethernet;

  let settings = loadSettings();
  let offlineTimer;

  function loadSettings() {
    try {
      return { ...DEFAULTS, ...JSON.parse(localStorage.getItem(STORAGE_KEY) || '{}') };
    } catch {
      return { ...DEFAULTS };
    }
  }

  function saveSettings() {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(settings));
    } catch {
      // Настройки останутся только до перезапуска
    }
    renderButton();
  }

  function getCurrentUserId() {
    return store?.getGlobal?.()?.currentUserId;
  }

  // Встроенный режим клиента появляется после загрузки приложения, поэтому берём его лениво
  function getBuiltinGhost() {
    return window.taurigramGhost;
  }

  function isGhostEnabled() {
    const builtin = getBuiltinGhost();
    return builtin ? builtin.isEnabled(getCurrentUserId()) : settings.enabled;
  }

  function setGhostEnabled(isEnabled) {
    settings.enabled = isEnabled;
    getBuiltinGhost()?.setEnabled(isEnabled, getCurrentUserId());
    saveSettings();
    // Без этого уже видимые сообщения отметятся прочитанными только при следующей прокрутке
    if (!isEnabled) readChat(getCurrentChatId());
  }

  function isActive(option) {
    return isGhostEnabled() && settings[option];
  }

  function notify(message) {
    store?.getActions?.()?.showNotification?.({ message });
  }

  // --- Перехват запросов ---

  api.beforeRequest((method, args) => {
    const params = args?.[0];
    const isForced = Boolean(params?.force);

    switch (method) {
      case 'markMessageListRead':
      case 'markMessagesRead':
        if (isActive('dontReadMessages') && !isForced) return false;
        break;
      case 'viewStory':
      case 'markStoryRead':
        if (isActive('dontReadStories')) return false;
        break;
      case 'sendMessageAction':
        if (isActive('dontSendTyping')) return false;
        break;
      case 'updateIsOnline':
        if (params === true && isActive('dontSendOnline')) return false;
        break;
      default:
        break;
    }

    if (!ACTIVITY_METHODS.has(method)) return undefined;

    if (method === 'sendMessage' && params?.chat?.id && isActive('readOnSend')) {
      readChat(params.chat.id);
    }

    scheduleOffline();
    return undefined;
  });

  // После действия сервер ставит «в сети» — сразу отправляем «не в сети»
  function scheduleOffline() {
    if (!isActive('sendOfflineAfterOnline')) return;
    clearTimeout(offlineTimer);
    offlineTimer = setTimeout(() => {
      store?.getActions?.()?.updateIsOnline?.({ isOnline: false });
    }, OFFLINE_DELAY_MS);
  }

  // --- Ручное прочтение ---

  function getCurrentChatId() {
    const global = store?.getGlobal?.();
    const tabs = Object.values(global?.byTabId || {});
    for (const tab of tabs) {
      const lists = tab?.messageLists;
      if (lists?.length) return lists[lists.length - 1].chatId;
    }
    return undefined;
  }

  function readChat(chatId) {
    const global = store?.getGlobal?.();
    const maxId = global?.chats?.lastMessageIds?.all?.[chatId];
    if (!chatId || !maxId) return false;
    // `force: true` проходит и через этот плагин, и через встроенный режим призрака
    store.getActions().forceMarkMessageListRead({ chatId, maxId });
    return true;
  }

  function readCurrentChat() {
    const chatId = getCurrentChatId();
    notify(readChat(chatId) ? '👻 Чат отмечен прочитанным' : '👻 Откройте чат, чтобы прочитать его');
  }

  // --- Горячие клавиши ---

  on('keydown', ({ key, ctrl, shift, alt }) => {
    const lower = (key || '').toLowerCase();
    if (ctrl && shift && (lower === 'g' || lower === 'п')) {
      toggleEnabled();
    } else if (alt && !ctrl && (lower === 'r' || lower === 'к')) {
      readCurrentChat();
    }
  });

  function toggleEnabled() {
    const isEnabled = !isGhostEnabled();
    setGhostEnabled(isEnabled);
    notify(isEnabled ? '👻 Режим призрака включён' : 'Режим призрака выключен');
    if (!isEnabled) clearTimeout(offlineTimer);
  }

  // --- Интерфейс ---

  addStyle(`
    #${BUTTON_ID} {
      flex-shrink: 0;
      width: 2.5rem;
      height: 2.5rem;
      margin-inline-start: 0.25rem;
      border: none;
      border-radius: 50%;
      background: transparent;
      color: var(--color-text-secondary);
      font-size: 1.25rem;
      line-height: 1;
      cursor: pointer;
      opacity: 0.45;
      filter: grayscale(1);
      transition: opacity 0.15s, filter 0.15s, background-color 0.15s;
    }
    #${BUTTON_ID}:hover { background: var(--color-interactive-element-hover, rgba(127,127,127,0.12)); }
    #${BUTTON_ID}.is-active { opacity: 1; filter: none; }

    #${PANEL_ID} {
      position: fixed;
      inset: 0;
      z-index: 100000;
      display: flex;
      align-items: center;
      justify-content: center;
      background: rgba(0, 0, 0, 0.45);
    }
    #${PANEL_ID} .ghost-card {
      width: min(26rem, calc(100vw - 2rem));
      max-height: calc(100vh - 4rem);
      overflow-y: auto;
      padding: 1.25rem;
      border-radius: var(--border-radius-default-small, 1rem);
      background: var(--color-background);
      color: var(--color-text);
      box-shadow: 0 0.5rem 2rem rgba(0, 0, 0, 0.35);
    }
    #${PANEL_ID} .ghost-title {
      display: flex;
      align-items: center;
      gap: 0.5rem;
      margin: 0 0 1rem;
      font-size: 1.125rem;
      font-weight: 600;
    }
    #${PANEL_ID} .ghost-row {
      display: flex;
      align-items: center;
      gap: 0.75rem;
      padding: 0.5rem 0;
      cursor: pointer;
    }
    #${PANEL_ID} .ghost-row.is-main { border-bottom: 1px solid var(--color-borders); margin-bottom: 0.25rem; }
    #${PANEL_ID} .ghost-row.is-disabled { opacity: 0.45; }
    #${PANEL_ID} .ghost-text { flex: 1; min-width: 0; }
    #${PANEL_ID} .ghost-label { font-size: 0.9375rem; }
    #${PANEL_ID} .ghost-hint { font-size: 0.8125rem; color: var(--color-text-secondary); }
    #${PANEL_ID} input[type="checkbox"] { width: 1.125rem; height: 1.125rem; accent-color: var(--color-primary); }
    #${PANEL_ID} .ghost-actions { display: flex; gap: 0.5rem; margin-top: 1rem; }
    #${PANEL_ID} .ghost-btn {
      flex: 1;
      padding: 0.625rem;
      border: none;
      border-radius: 0.75rem;
      background: var(--color-background-secondary);
      color: var(--color-text);
      font-size: 0.875rem;
      cursor: pointer;
    }
    #${PANEL_ID} .ghost-btn.is-primary { background: var(--color-primary); color: #fff; }
    #${PANEL_ID} .ghost-keys { margin-top: 0.75rem; font-size: 0.75rem; color: var(--color-text-secondary); }
  `, STYLE_ID);

  function renderButton() {
    const button = document.getElementById(BUTTON_ID);
    if (!button) return;
    const isEnabled = isGhostEnabled();
    button.classList.toggle('is-active', isEnabled);
    button.title = isEnabled
      ? 'Режим призрака включён (клик — выключить, правый клик — настройки)'
      : 'Режим призрака выключен (клик — включить, правый клик — настройки)';
  }

  function ensureButton() {
    const header = document.getElementById('LeftMainHeader');
    if (!header || document.getElementById(BUTTON_ID)) return;
    const button = document.createElement('button');
    button.id = BUTTON_ID;
    button.type = 'button';
    button.textContent = '👻';
    button.addEventListener('click', toggleEnabled);
    button.addEventListener('contextmenu', (e) => {
      e.preventDefault();
      openPanel();
    });
    header.appendChild(button);
    renderButton();
    subscribeToBuiltin();
  }

  // Переключение режима в настройках клиента сразу отражается на кнопке
  let isSubscribedToBuiltin = false;
  function subscribeToBuiltin() {
    const builtin = getBuiltinGhost();
    if (isSubscribedToBuiltin || !builtin?.subscribe) return;
    isSubscribedToBuiltin = true;
    builtin.subscribe(renderButton);
  }

  function openPanel() {
    document.getElementById(PANEL_ID)?.remove();
    const overlay = document.createElement('div');
    overlay.id = PANEL_ID;
    overlay.addEventListener('click', (e) => {
      if (e.target === overlay) overlay.remove();
    });

    const card = document.createElement('div');
    card.className = 'ghost-card';
    card.innerHTML = '<div class="ghost-title">👻 Режим призрака</div>';

    card.appendChild(createRow('enabled', 'Включить режим призрака', 'Общий со встроенным режимом в настройках', true));
    OPTIONS.forEach(([key, label, hint]) => card.appendChild(createRow(key, label, hint)));

    const actions = document.createElement('div');
    actions.className = 'ghost-actions';
    actions.appendChild(createButton('Прочитать открытый чат', () => {
      overlay.remove();
      readCurrentChat();
    }));
    actions.appendChild(createButton('Готово', () => overlay.remove(), true));
    card.appendChild(actions);

    const keys = document.createElement('div');
    keys.className = 'ghost-keys';
    keys.textContent = 'Ctrl+Shift+G — вкл/выкл · Alt+R — прочитать открытый чат · правый клик по 👻 — это окно';
    card.appendChild(keys);

    overlay.appendChild(card);
    document.body.appendChild(overlay);
  }

  function createRow(key, label, hint, isMain) {
    const row = document.createElement('label');
    row.className = `ghost-row${isMain ? ' is-main' : ''}${!isMain && !isGhostEnabled() ? ' is-disabled' : ''}`;
    const text = document.createElement('div');
    text.className = 'ghost-text';
    const labelEl = document.createElement('div');
    labelEl.className = 'ghost-label';
    labelEl.textContent = label;
    const hintEl = document.createElement('div');
    hintEl.className = 'ghost-hint';
    hintEl.textContent = hint;
    text.append(labelEl, hintEl);

    const input = document.createElement('input');
    input.type = 'checkbox';
    input.checked = isMain ? isGhostEnabled() : Boolean(settings[key]);
    input.addEventListener('change', () => {
      if (isMain) {
        setGhostEnabled(input.checked);
        openPanel();
        return;
      }
      settings[key] = input.checked;
      saveSettings();
    });
    row.append(text, input);
    return row;
  }

  function createButton(text, onClick, isPrimary) {
    const button = document.createElement('button');
    button.type = 'button';
    button.className = `ghost-btn${isPrimary ? ' is-primary' : ''}`;
    button.textContent = text;
    button.addEventListener('click', onClick);
    return button;
  }

  // Шапка списка чатов перерисовывается клиентом — возвращаем кнопку на место
  let isEnsureQueued = false;
  new MutationObserver(() => {
    if (isEnsureQueued) return;
    isEnsureQueued = true;
    requestAnimationFrame(() => {
      isEnsureQueued = false;
      ensureButton();
    });
  }).observe(document.documentElement, { childList: true, subtree: true });
  ensureButton();

  log(`[${PLUGIN_ID}] загружен`);
})();
