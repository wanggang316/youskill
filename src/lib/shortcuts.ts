import { derived, get } from "svelte/store";
import { settings, updateSettings } from "./stores/settings";

/** Top bar actions that accept a shortcut, in display order. Keep in sync with `SHORTCUT_ACTIONS` in Rust. */
export const SHORTCUT_ACTIONS = [
  "library",
  "instructions",
  "projects",
  "market",
  "import",
  "search",
  "settings",
] as const;

export type ShortcutAction = (typeof SHORTCUT_ACTIONS)[number];

/**
 * A key combination such as `"m"` or `"Mod+,"`. Modifiers come first in the order
 * Mod, Ctrl, Alt, Shift. `Mod` is Command on macOS and Control elsewhere; `Ctrl` is
 * the Control key on macOS only.
 */
export type Binding = string;

export const DEFAULT_SHORTCUTS: Record<ShortcutAction, Binding> = {
  library: "1",
  instructions: "2",
  projects: "3",
  market: "m",
  import: "n",
  search: "Mod+p",
  settings: "Mod+,",
};

/** i18n key of each action's name. */
export const SHORTCUT_LABEL_KEYS: Record<ShortcutAction, string> = {
  library: "sidebar.library",
  instructions: "sidebar.instructions",
  projects: "sidebar.projects",
  market: "sidebar.market",
  import: "import.title",
  search: "search.title",
  settings: "header.settings",
};

export const isMac =
  typeof navigator !== "undefined" &&
  /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);

/** The key that shows the shortcut hints when held down. */
export const HINT_KEY = isMac ? "Meta" : "Control";

/** Combinations that the app or the system already uses. */
const RESERVED: Binding[] = [
  "Mod+[",
  "Mod+]",
  "Mod+a",
  "Mod+c",
  "Mod+h",
  "Mod+m",
  "Mod+q",
  "Mod+v",
  "Mod+w",
  "Mod+x",
  "Mod+z",
  "Mod+Shift+z",
];

const MODIFIER_KEYS = new Set(["Meta", "Control", "Alt", "Shift", "CapsLock", "Fn", "OS"]);

type ParsedBinding = { mod: boolean; ctrl: boolean; alt: boolean; shift: boolean; key: string };

const parseBinding = (binding: Binding): ParsedBinding => {
  // The key itself can be "+", so the last character never separates.
  const cut = binding.lastIndexOf("+", binding.length - 2);
  const key = binding.slice(cut + 1);
  const modifiers = new Set(cut < 0 ? [] : binding.slice(0, cut).split("+"));
  return {
    mod: modifiers.has("Mod"),
    ctrl: modifiers.has("Ctrl"),
    alt: modifiers.has("Alt"),
    shift: modifiers.has("Shift"),
    key,
  };
};

/** The key without modifiers, from the physical key so that Shift and Option do not change it. */
const eventKey = (event: KeyboardEvent): string => {
  if (event.code.startsWith("Key")) return event.code.slice(3).toLowerCase();
  if (event.code.startsWith("Digit")) return event.code.slice(5);
  return event.key.length === 1 ? event.key.toLowerCase() : event.key;
};

/** The binding that a key press makes, or null for a modifier pressed alone. */
export const eventToBinding = (event: KeyboardEvent): Binding | null => {
  if (MODIFIER_KEYS.has(event.key)) return null;
  const parts: string[] = [];
  if (isMac ? event.metaKey : event.ctrlKey) parts.push("Mod");
  if (isMac && event.ctrlKey) parts.push("Ctrl");
  if (event.altKey) parts.push("Alt");
  if (event.shiftKey) parts.push("Shift");
  parts.push(eventKey(event));
  return parts.join("+");
};

/** True when the binding types text, so it must not fire inside a text field. */
export const isBareBinding = (binding: Binding): boolean => {
  const { mod, ctrl, alt } = parseBinding(binding);
  return !mod && !ctrl && !alt;
};

export const isReservedBinding = (binding: Binding): boolean => RESERVED.includes(binding);

const KEY_SYMBOLS: Record<string, string> = {
  ArrowUp: "↑",
  ArrowDown: "↓",
  ArrowLeft: "←",
  ArrowRight: "→",
  Enter: "↩",
  Backspace: "⌫",
  Delete: "⌦",
  Escape: "Esc",
  Tab: "⇥",
  " ": "Space",
  PageUp: "PgUp",
  PageDown: "PgDn",
};

/** Keys to show for a binding, with the system symbols on macOS (⌘ ⌃ ⌥ ⇧). */
export const bindingKeys = (binding: Binding): string[] => {
  const { mod, ctrl, alt, shift, key } = parseBinding(binding);
  const keys: string[] = [];
  if (isMac) {
    if (ctrl) keys.push("⌃");
    if (alt) keys.push("⌥");
    if (shift) keys.push("⇧");
    if (mod) keys.push("⌘");
  } else {
    if (mod) keys.push("Ctrl");
    if (alt) keys.push("Alt");
    if (shift) keys.push("Shift");
  }
  keys.push(KEY_SYMBOLS[key] ?? key);
  return keys;
};

/** Value for `aria-keyshortcuts`. */
export const ariaKeyShortcut = (binding: Binding): string => {
  const { mod, ctrl, alt, shift, key } = parseBinding(binding);
  const parts: string[] = [];
  if (mod) parts.push(isMac ? "Meta" : "Control");
  if (ctrl) parts.push("Control");
  if (alt) parts.push("Alt");
  if (shift) parts.push("Shift");
  parts.push(key === " " ? "Space" : key);
  return parts.join("+");
};

/** Effective binding of every action: the user's change, else the default. */
export const shortcuts = derived(settings, ($settings) => {
  const overrides = $settings.shortcuts ?? {};
  return Object.fromEntries(
    SHORTCUT_ACTIONS.map((action) => [action, overrides[action] || DEFAULT_SHORTCUTS[action]])
  ) as Record<ShortcutAction, Binding>;
});

/** The action bound to a key press, if any. */
export const matchShortcut = (
  event: KeyboardEvent,
  bindings: Record<ShortcutAction, Binding>
): ShortcutAction | null => {
  const binding = eventToBinding(event);
  if (!binding) return null;
  return SHORTCUT_ACTIONS.find((action) => bindings[action] === binding) ?? null;
};

/** Stores only the changes; setting the default again removes the change. */
export const setShortcut = (action: ShortcutAction, binding: Binding | null) => {
  const next = { ...get(settings).shortcuts };
  if (binding === null || binding === DEFAULT_SHORTCUTS[action]) delete next[action];
  else next[action] = binding;
  return updateSettings({ shortcuts: next });
};

export const resetShortcuts = () => updateSettings({ shortcuts: {} });
