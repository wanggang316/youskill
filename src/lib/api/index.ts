/**
 * API 统一封装层
 *
 * 提供对 Tauri IPC 命令的封装，统一错误处理和类型安全
 */

import { invoke } from "@tauri-apps/api/core";

/**
 * 自定义错误类
 */
export class AppError extends Error {
  constructor(
    public code: string,
    message: string,
    public hint?: string
  ) {
    super(message);
    this.name = "AppError";
  }
}

/**
 * 统一的 IPC 调用封装
 */
export async function apiCall<T>(command: string, payload?: unknown): Promise<T> {
  try {
    return await invoke<T>(command, payload as Record<string, unknown>);
  } catch (error: unknown) {
    // 处理 Tauri IPC 错误
    if (error && typeof error === "object") {
      // 如果错误包含我们的 AppError 结构
      if ("code" in error && "message" in error) {
        throw new AppError(
          error.code as string,
          error.message as string,
          "hint" in error ? (error.hint as string) : undefined
        );
      }
      // 如果是包装在其他结构中的错误
      if ("error" in error) {
        const err = error.error as { code?: string; message?: string; hint?: string };
        if (err.code && err.message) {
          throw new AppError(err.code, err.message, err.hint);
        }
      }
    }

    // 处理其他类型的错误
    throw new AppError(
      "IPC_ERROR",
      error instanceof Error ? error.message : typeof error === "string" ? error : "IPC 调用失败",
      "请检查应用状态或重新启动"
    );
  }
}

// 导出 API 模块
export * from "./agent-apps";
export * from "./cli";
export * from "./github";
export * from "./hub";
export * from "./instructions";
export * from "./library-sync";
export * from "./search";
export * from "./skills";
export * from "./settings";
export * from "./user-projects";
