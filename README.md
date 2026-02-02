<div align="center">
  <img src="src-tauri/icons/icon.png" width="240" alt="reaction-cut-rust logo" />

  <h1>reaction-cut-rust</h1>
  <p>面向 Bilibili 录播的下载、剪辑、分段、投稿一体化桌面工具</p>

  <!-- 徽章占位，可按需替换为真实仓库链接 -->
  <!--
  <div>
    <img src="https://img.shields.io/github/stars/your-org/your-repo" />
    <img src="https://img.shields.io/github/v/release/your-org/your-repo" />
    <img src="https://img.shields.io/github/license/your-org/your-repo" />
  </div>
  -->
</div>

<hr />

## 项目描述

reaction-cut-rust 是一个基于 Tauri 的跨平台桌面应用，聚焦 Bilibili 录播场景：

- 录播订阅与自动录制
- 多分 P 下载
- 剪辑、合并、分段工作流
- 投稿、更新与失败重试
- 可选百度网盘同步

## 技术栈

- **前端**：React 18 + Vite + Tailwind CSS
- **桌面端**：Tauri 2
- **后端**：Rust
- **数据**：SQLite
- **多媒体/下载**：FFmpeg / FFprobe / aria2c / BaiduPCS-Go（内置二进制）

## 实现功能

| 模块 | 状态 | 说明 |
| ---- | ---- | ---- |
| 直播录制 | ✅ 已完成 | 支持订阅、自动录制与分段策略 |
| 视频下载 | ✅ 已完成 | 分 P 选择、支持多分辨率/编码/格式 |
| 剪辑 | ✅ 已完成 | 支持起止时间裁剪与校验 |
| 合并 | ✅ 已完成 | 多段视频合并成投稿素材 |
| 分段 | ✅ 已完成 | 按配置时长自动切段 |
| 投稿 | ✅ 已完成 | 提交、更新、失败重试与状态追踪 |
| 同步 | ✅ 已完成 | 可选百度网盘同步 |
| 工具 | ✅ 已完成 | 内置转封装与辅助工具 |

## 使用需求

- Node.js 18+
- pnpm 8+
- Rust stable
- Tauri 依赖（macOS 需安装 Xcode Command Line Tools）
- 需要 Bilibili 登录态（扫码/密码/短信等）
- 如启用百度网盘同步，需要百度网盘登录

> [!IMPORTANT]
> 资源解析与投稿依赖账号权限，请确保账号具备相应内容访问权限。

## 快速开始

开发模式：

```bash
pnpm install
pnpm tauri dev
```

仅启动前端：

```bash
pnpm dev
```

构建与打包：

```bash
pnpm tauri build --bundles dmg
```

> macOS DMG 生成依赖 `hdiutil`，需在非沙箱环境下执行。

## 运行数据位置（macOS）

- 数据目录：`~/Library/Application Support/com.tbw.reaction-cut-rust/`
- 数据库：`reaction-cut-rust.sqlite3`
- 日志：`app_debug.log` / `auth_debug.log` / `panic_debug.log`

## 界面预览

> 请在此处补充项目截图

| 首页 | 下载/投稿 |
| ---- | -------- |
| ![截图占位](.github/screenshot-home.png) | ![截图占位](.github/screenshot-download.png) |

## 贡献

请参考 `CONTRIBUTING.md`。

## 许可与声明

- 协议：见 `LICENSE`
- 第三方组件：见 `THIRD_PARTY_NOTICES.md`

> 本项目仅用于学习与技术研究，请遵守平台服务协议及相关法律法规。
