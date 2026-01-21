import { openUrl } from "@tauri-apps/plugin-opener";
import pkg from "../../package.json";

const actionLinks = [
  {
    buttonText: "作者主页",
    url: "https://space.bilibili.com/82679456?spm_id_from=333.788.0.0",
  },
  {
    buttonText: "GitHub仓库",
    url: "https://github.com/tianbowen14300/reaction-cut-rust",
  },
  {
    buttonText: "视频教程",
    url: "https://www.bilibili.com/video/BV1P4r9BfESr/?spm_id_from=333.1387.homepage.video_card.click",
  },
  {
    buttonText: "反馈地址",
    url: "https://github.com/tianbowen14300/reaction-cut-rust/issues",
  },
];


const ExternalLinkIcon = () => (
  <svg
    viewBox="0 0 24 24"
    width="14"
    height="14"
    aria-hidden="true"
    focusable="false"
  >
    <path
      d="M14 3h7v7h-2V6.41l-9.29 9.3-1.42-1.42 9.3-9.29H14V3z"
      fill="currentColor"
    />
    <path
      d="M5 5h6v2H7v10h10v-4h2v6H5V5z"
      fill="currentColor"
    />
  </svg>
);

const openExternal = async (url) => {
  if (!url) {
    return;
  }
  try {
    await openUrl(url);
  } catch (_) {}
};

export default function AboutSection() {
  const version = pkg?.version || "unknown";

  return (
    <div className="space-y-3">
      <div className="panel p-4 space-y-2">
        <div className="space-y-1">
          <div className="text-lg font-semibold text-[var(--content-color)]">Reaction Cut</div>
          <div className="desc">
            介绍：支持直播间订阅与手动/自动录制（含弹幕录制配置），支持视频下载（分P选择、多视频、下载+投稿），提供投稿任务的剪辑/合并/分段/更新与重试管理，并内置 FLV 转 MP4 转封装工具。
          </div>
        </div>
        <div className="space-y-1 text-sm text-[var(--content-color)]">
          <div>版本：{version}</div>
          <div className="flex flex-wrap gap-2">
            {actionLinks.map((item) => (
              <button
                key={item.buttonText}
                className="inline-flex items-center gap-2 rounded-full border border-[var(--split-color)] bg-[var(--solid-button-color)] px-3 py-1 text-xs text-[var(--content-color)] transition hover:border-[var(--primary-color)]"
                onClick={() => openExternal(item.url)}
                type="button"
              >
                <ExternalLinkIcon />
                {item.buttonText}
              </button>
            ))}
          </div>
          <div>本软件为开源软件，许可证：GPLv3</div>
          <div className="text-[var(--desc-color)]">
            此软件为公益免费项目。如果你付费购买了此软件，你可能被骗了。
          </div>
          <div className="text-[var(--desc-color)]">
            觉得好用的话，在 GitHub 给这个项目点个 Star 吧！
          </div>
        </div>
      </div>

    </div>
  );
}
