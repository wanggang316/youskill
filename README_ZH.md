# YouSkill

[![Version](https://img.shields.io/github/v/release/wanggang316/you-skill?label=Version)](https://github.com/wanggang316/you-skill/releases)
[![Platform](https://img.shields.io/badge/Platform-Windows%20%7C%20macOS-lightgrey.svg)](https://github.com/wanggang316/you-skill/releases)
[![Built%20with%20Tauri](https://img.shields.io/badge/Built%20with-Tauri%202-orange.svg)](https://tauri.app/)

[English](README.md) | 中文

**你的 Agent 资产管理伙伴。**

YouSkill 是一个轻量的桌面应用，统一管理各个 Agent 和项目中的 [Agent Skills](https://agentskills.io/) 与指令文件。中心库连接来源和安装位置，每份副本的变更清晰可见，保留哪一版由你决定。基于 Tauri 构建，快速、轻量。

## 管理 Skill 的整个生命周期

1. **导入一次。** 浏览内置市场、粘贴 GitHub 地址，或选择 `.zip` / `.skill` 压缩包、本地文件夹。
2. **装到需要的地方。** 选择用户级或某个项目，勾选 Agent，再选择复制或软链接。内置 Claude Code、Codex、Cursor、OpenCode 等常见 Agent，也可通过 Skill 目录添加自定义 Agent。
3. **追踪每份副本。** 查看各个项目和 Agent 安装了哪些 Skill，在同步前识别落后版本、本地修改和冲突。
4. **决定如何更新。** 检查来源更新、查看本地差异，再选择将本地副本采纳到中心库，或从中心库推送到安装位置。

## 功能概览

- **统一中心库。** 按来源组织 Skill，查看完整文件、版本和安装位置；扫描已有 Agent 或项目目录，将 Skill 纳入中心库。
- **共享指令。** 集中管理 `AGENTS.md` 和 `CLAUDE.md` 模板，应用到多个 Agent 和项目。
- **翻译与双语阅读。** 通过大模型翻译，用熟悉的语言对照原文阅读 Skill。
- **内置 Skill 市场。** 按分类浏览，一键导入，已在库中的 Skill 会显示标记。
- **命令行工作流。** 通过 `youskill` 命令行导入、安装、查看和同步 Skill 与指令，也可让 Agent 帮你管理。
- **日常管理。** 把中心库备份到 GitHub 私有仓库并在其他电脑上恢复，切换主题和界面语言。

存储布局与变更追踪模型见 [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)，命令行用法见 [docs/CLI.md](docs/CLI.md)。

## 预览

### Skill 库

按来源浏览 Skill，查看版本、完整文件和安装位置。

![Skill 库：来源分组、版本信息和安装位置](./public/shot-skills.webp)

### 指令

统一管理可复用的指令模板，查看使用它们的 Agent 和项目。

![指令模板及其安装的 AGENTS.md 和 CLAUDE.md 文件](./public/shot-instructions.webp)

### 项目

在一个视图中查看项目的 Agent、指令文件和已安装 Skill。

![项目概览：Agent、指令文件和已安装 Skill](./public/shot-projects.webp)

## 下载安装

### Homebrew（macOS）

```bash
# App
brew install --cask wanggang316/tap/youskill

# Command line tool only
brew install wanggang316/tap/youskill
```

App 会自动更新。App 内置 `youskill` 命令行工具，可在「设置 > 命令行工具」中一键安装，使用 App 时无需再装 formula。

### 安装包

从 [Releases](https://github.com/wanggang316/you-skill/releases) 页面下载对应平台的安装包：

- **macOS**: `YouSkill_x.x.x_aarch64.dmg` (Apple Silicon) / `YouSkill_x.x.x_x64.dmg` (Intel)
- **Windows**: `YouSkill_x.x.x_x64-setup.exe`

## 技术栈

- **Tauri v2**
- **Svelte 5**
- **Vite**
- **Tailwind CSS**
- **Lucide Icons**

## 项目结构

- `src/`: 前端核心目录
- `src-tauri/`: Rust 后端与打包配置
- `scripts/`: 发布与维护脚本

## 开发

### 前置要求

- [Node.js](https://nodejs.org/) (建议 18+)
- [Rust](https://www.rust-lang.org/tools/install)
- [Tauri 依赖环境](https://tauri.app/start/prerequisites/)

### 安装依赖

```bash
npm install
```

### 本地运行

```bash
npm run tauri -- dev
```

### 构建

```bash
npm run build
npm run tauri -- build
```

### 发布

1. 在 [CHANGELOG.md](CHANGELOG.md) 的 `[Unreleased]` 部分填写本次更新内容
2. 执行发布脚本：
   ```bash
   npm run release <version>
   ```
   示例：`npm run release 0.9.0`

### 开发规则

[AGENTS.md](AGENTS.md)

## 路线图

- [x] 项目级的 Skill 管理
- [x] 中心化 Skill 库，支持变更追踪与文件夹扫描
- [x] Skill 详情，支持快速查看所有 Skill 文件
- [ ] 备份到 GitHub，实现多设备同步
- [ ] 基于 AI 的 FindSkill

## 贡献

[CONTRIBUTING.md](CONTRIBUTING.md)

## 许可证

MIT
