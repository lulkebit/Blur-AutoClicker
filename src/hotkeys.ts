const MODIFIER_ALIASES: Record<string, string> = {
  control: "ctrl",
  ctrl: "ctrl",
  option: "alt",
  alt: "alt",
  shift: "shift",
  meta: "super",
  command: "super",
  cmd: "super",
  super: "super",
  win: "super",
};

const MODIFIER_KEYS = new Set([
  "control",
  "ctrl",
  "shift",
  "alt",
  "meta",
  "os",
  "altgraph",
]);

const SHIFTED_SYMBOL_BASE_MAP: Record<string, string> = {
  "?": "/",
  ":": ";",
  "\"": "'",
  "{": "[",
  "}": "]",
  "|": "\\",
  "+": "=",
  "_": "-",
  "~": "`",
  ">": "<",
};

type LayoutMapLike = {
  get(code: string): string | undefined;
};

let layoutMapPromise: Promise<LayoutMapLike | null> | null = null;
const IS_MAC = /mac|iphone|ipad|ipod/i.test(
  `${navigator.userAgent} ${navigator.platform}`,
);

function normalizeModifierToken(token: string): string | null {
  return MODIFIER_ALIASES[token.trim().toLowerCase()] ?? null;
}

function normalizeNamedKey(key: string): string | null {
  const lower = key.toLowerCase();

  const keyMap: Record<string, string> = {
    enter: "enter",
    tab: "tab",
    spacebar: "space",
    backspace: "backspace",
    delete: "delete",
    insert: "insert",
    home: "home",
    end: "end",
    pageup: "pageup",
    pagedown: "pagedown",
    arrowup: "up",
    arrowdown: "down",
    arrowleft: "left",
    arrowright: "right",
    // Mouse buttons
    mouseleft: "mouseleft",
    mouse1: "mouseleft",
    mouseright: "mouseright",
    mouse2: "mouseright",
    mousemiddle: "mousemiddle",
    mouse3: "mousemiddle",
    scrollbutton: "mousemiddle",
    middleclick: "mousemiddle",
    mouse4: "mouse4",
    mouseback: "mouse4",
    xbutton1: "mouse4",
    mouse5: "mouse5",
    mouseforward: "mouse5",
    xbutton2: "mouse5",
    // Scroll wheel
    scrollup: "scrollup",
    wheelup: "scrollup",
    scrolldown: "scrolldown",
    wheeldown: "scrolldown",
  };

  if (/^f\d{1,2}$/i.test(key)) {
    return lower;
  }

  return keyMap[lower] ?? null;
}

function displayTokenFromStoredValue(token: string, layoutMap: LayoutMapLike | null): string {
  const trimmed = token.trim();
  if (!trimmed) return trimmed;

  if (trimmed === "IntlBackslash") {
    return layoutMap?.get("IntlBackslash") ?? "<";
  }

  if (/^Key[A-Z]$/.test(trimmed)) {
    const mapped = layoutMap?.get(trimmed);
    if (mapped) return mapped;
    return trimmed.slice(3).toLowerCase();
  }

  if (/^Digit[0-9]$/.test(trimmed)) {
    return trimmed.slice(5);
  }

  const lower = trimmed.toLowerCase();
  const namedDisplayMap: Record<string, string> = {
    up: "Up",
    down: "Down",
    left: "Left",
    right: "Right",
    pageup: "Page Up",
    pagedown: "Page Down",
    backspace: "Backspace",
    delete: "Delete",
    insert: "Insert",
    home: "Home",
    end: "End",
    enter: "Enter",
    tab: "Tab",
    space: "Space",
    escape: "Esc",
    esc: "Esc",
    // Mouse buttons
    mouseleft: "Mouse Left",
    mouseright: "Mouse Right",
    mousemiddle: "Scroll Button",
    mouse4: "Mouse Back",
    mouse5: "Mouse Forward",
    // Scroll wheel
    scrollup: "Scroll Up",
    scrolldown: "Scroll Down",
  };

  if (namedDisplayMap[lower]) {
    return namedDisplayMap[lower];
  }

  return trimmed;
}

function normalizeStoredMainKey(token: string, layoutMap: LayoutMapLike | null): string {
  const trimmed = token.trim();
  if (!trimmed) return trimmed;

  if (trimmed === "IntlBackslash") {
    return "IntlBackslash";
  }

  if (/^Key[A-Z]$/.test(trimmed)) {
    const mapped = layoutMap?.get(trimmed);
    return mapped ? mapped.toLowerCase() : trimmed.slice(3).toLowerCase();
  }

  if (/^Digit[0-9]$/.test(trimmed)) {
    return trimmed.slice(5);
  }

  const lower = trimmed.toLowerCase();
  if (lower === "<" || lower === ">") {
    return "IntlBackslash";
  }

  if (SHIFTED_SYMBOL_BASE_MAP[trimmed]) {
    return SHIFTED_SYMBOL_BASE_MAP[trimmed];
  }

  return normalizeNamedKey(trimmed) ?? lower;
}

export async function getKeyboardLayoutMap(): Promise<LayoutMapLike | null> {
  if (!layoutMapPromise) {
    const keyboard = (navigator as Navigator & {
      keyboard?: { getLayoutMap?: () => Promise<LayoutMapLike> };
    }).keyboard;

    layoutMapPromise = keyboard?.getLayoutMap
      ? keyboard.getLayoutMap().catch(() => null)
      : Promise.resolve(null);
  }

  return layoutMapPromise;
}

export async function canonicalizeHotkeyForBackend(value: string): Promise<string> {
  const layoutMap = await getKeyboardLayoutMap();
  return canonicalizeHotkeyString(value, layoutMap);
}

export function captureHotkey(event: {
  key: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
}): string | null {
  const lower = event.key.toLowerCase();

  if (MODIFIER_KEYS.has(lower)) return null;
  if (lower === "escape") return null;
  if (event.key === " ") return "space";

  const normalizedNamedKey = normalizeNamedKey(event.key);
  const mainKey =
    normalizedNamedKey ??
    (SHIFTED_SYMBOL_BASE_MAP[event.key] ?? (event.key.length === 1 ? lower : null));

  if (!mainKey) return null;

  const parts: string[] = [];
  if (event.ctrlKey) parts.push("ctrl");
  if (event.altKey) parts.push("alt");
  if (event.shiftKey) parts.push("shift");
  if (event.metaKey) parts.push("super");
  parts.push(mainKey);
  return parts.join("+");
}

/**
 * Capture a mouse button (middle, right, side-buttons) as a hotkey.
 * Returns null for left-click (button 0) since that's used for UI interaction,
 * and null for plain right-click (button 2) to avoid context-menu confusion.
 */
export function captureMouseHotkey(event: {
  button: number;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;},
  clickerMouseButton?: string 
): string | null {
  const mouseMap: Record<number, string> = {
    0: "mouseleft",
    1: "mousemiddle",
    2: "mouseright",
    3: "mouse4",
    4: "mouse5",
  };

  const mainKey = mouseMap[event.button];
  if (!mainKey) return null; // left click (0) or unknown

  if (clickerMouseButton === "Left" && mainKey === "mouseleft") return null;
  if (clickerMouseButton === "Middle" && mainKey === "mousemiddle") return null;
  if (clickerMouseButton === "Right" && mainKey === "mouseright") return null;

  if (event.button === 0) { // allow Left click with modifier
    const hasModifier = event.ctrlKey || event.altKey || event.shiftKey || event.metaKey;
    if (!hasModifier) return null;
  }

  const parts: string[] = [];
  if (event.ctrlKey) parts.push("ctrl");
  if (event.altKey) parts.push("alt");
  if (event.shiftKey) parts.push("shift");
  if (event.metaKey) parts.push("super");
  parts.push(mainKey);
  return parts.join("+");
}

/**
 * Capture a scroll wheel direction as a hotkey.
 */
export function captureWheelHotkey(event: {
  deltaY: number;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
}): string | null {
  if (event.deltaY === 0) return null;

  const mainKey = event.deltaY < 0 ? "scrollup" : "scrolldown";

  const parts: string[] = [];
  if (event.ctrlKey) parts.push("ctrl");
  if (event.altKey) parts.push("alt");
  if (event.shiftKey) parts.push("shift");
  if (event.metaKey) parts.push("super");
  parts.push(mainKey);
  return parts.join("+");
}

export function formatHotkeyForDisplay(value: string, layoutMap: LayoutMapLike | null): string {
  if (!value) return "Click and press keys";

  return value
    .split("+")
    .map((part) => {
      const modifier = normalizeModifierToken(part);
      if (modifier) {
        if (modifier === "ctrl") return "Ctrl";
        if (modifier === "alt") return IS_MAC ? "Option" : "Alt";
        if (modifier === "shift") return "Shift";
        return IS_MAC ? "Cmd" : "Super";
      }

      const display = displayTokenFromStoredValue(part, layoutMap);
      return display.length === 1 ? display.toUpperCase() : display;
    })
    .join(" + ");
}

function canonicalizeHotkeyString(value: string, layoutMap: LayoutMapLike | null): string {
  const parts: string[] = [];
  let mainKey: string | null = null;

  for (const rawPart of value.split("+")) {
    const part = rawPart.trim();
    if (!part) continue;

    const modifier = normalizeModifierToken(part);
    if (modifier) {
      if (!parts.includes(modifier)) {
        parts.push(modifier);
      }
      continue;
    }

    mainKey = normalizeStoredMainKey(part, layoutMap);
  }

  if (mainKey) {
    parts.push(mainKey);
  }

  return parts.join("+");
}
