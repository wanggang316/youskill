/**
 * Settings API
 *
 * 处理应用设置相关的 IPC 调用
 */

import { apiCall } from "./index";

export type TranslateDisplayMode = "bilingual" | "translation";
export type TranslateTextStyle = "none" | "dashed_underline";

export interface AppSettings {
  language: "en" | "zh";
  theme: "system" | "light" | "dark";
  /** Default install mode for new targets. */
  sync_mode: "symlink" | "copy";
  openrouter_api_key?: string | null;
  translate_target_language: string;
  translate_model: string;
  translate_display_mode: TranslateDisplayMode;
  translate_text_style: TranslateTextStyle;
  /** Changed key combinations by action; actions not listed use their default. */
  shortcuts: Record<string, string>;
}

export type EditableSettings = Pick<
  AppSettings,
  | "language"
  | "theme"
  | "sync_mode"
  | "openrouter_api_key"
  | "translate_target_language"
  | "translate_model"
  | "translate_display_mode"
  | "translate_text_style"
  | "shortcuts"
>;

/**
 * 获取应用设置
 */
export async function getSettings(): Promise<AppSettings> {
  return apiCall<AppSettings>("get_settings");
}

/**
 * 更新应用设置
 */
export async function updateSettings(settings: EditableSettings): Promise<AppSettings> {
  return apiCall<AppSettings>("update_settings", { settings });
}

export interface OpenRouterModelOption {
  id: string;
  name: string;
}

export async function listOpenRouterModels(search?: string): Promise<OpenRouterModelOption[]> {
  return apiCall<OpenRouterModelOption[]>("list_openrouter_models", {
    search: search?.trim() || null,
  });
}
