import { useMemo, useState } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { invokeCommand } from "../lib/tauri";

const toolboxTabs = [{ key: "remux", label: "格式转码" }];

const normalizePath = (path) => String(path || "").replace(/\\/g, "/");

const buildDefaultTarget = (sourcePath) => {
  const normalized = normalizePath(sourcePath);
  if (!normalized) {
    return "";
  }
  const lastSlash = normalized.lastIndexOf("/");
  const dir = lastSlash >= 0 ? normalized.slice(0, lastSlash + 1) : "";
  const base = lastSlash >= 0 ? normalized.slice(lastSlash + 1) : normalized;
  const baseName = base.replace(/\.[^.]+$/, "");
  return `${dir}${baseName}.mp4`;
};

const ensureMp4Extension = (path) => {
  if (!path) {
    return "";
  }
  return path.toLowerCase().endsWith(".mp4") ? path : `${path}.mp4`;
};

export default function ToolboxSection() {
  const [activeTab, setActiveTab] = useState("remux");
  const [sourcePath, setSourcePath] = useState("");
  const [targetPath, setTargetPath] = useState("");
  const [message, setMessage] = useState("");
  const [running, setRunning] = useState(false);

  const defaultTarget = useMemo(() => buildDefaultTarget(sourcePath), [sourcePath]);

  const handlePickSource = async () => {
    setMessage("");
    const selected = await open({
      multiple: false,
      directory: false,
      title: "选择 FLV 文件",
      filters: [{ name: "FLV", extensions: ["flv"] }],
    });
    if (typeof selected === "string") {
      const nextDefault = buildDefaultTarget(selected);
      setSourcePath(selected);
      setTargetPath((prev) => {
        if (!prev || prev === defaultTarget) {
          return nextDefault;
        }
        return prev;
      });
    }
  };

  const handlePickTarget = async () => {
    setMessage("");
    const selected = await save({
      title: "保存 MP4 文件",
      filters: [{ name: "MP4", extensions: ["mp4"] }],
      defaultPath: defaultTarget || undefined,
    });
    if (typeof selected === "string") {
      setTargetPath(ensureMp4Extension(selected));
    }
  };

  const handleRemux = async () => {
    setMessage("");
    if (!sourcePath.trim()) {
      setMessage("请选择 FLV 文件");
      return;
    }
    if (!targetPath.trim()) {
      setMessage("请选择输出路径");
      return;
    }
    setRunning(true);
    try {
      await invokeCommand("toolbox_remux", {
        payload: {
          sourcePath: sourcePath,
          targetPath: targetPath,
        },
      });
      setMessage("转封装完成");
    } catch (error) {
      setMessage(error?.message || "转封装失败");
    } finally {
      setRunning(false);
    }
  };

  return (
    <div className="flex gap-4">
      <div className="flex-1 min-w-0 space-y-4">
        <div className="panel p-4 space-y-3">
          <div className="space-y-1">
            <div className="text-lg font-semibold">格式转码</div>
            <div className="desc">
              基于 FFmpeg 转封装，仅支持 FLV 转 MP4，不进行重新编码。
            </div>
          </div>
          <div className="space-y-2">
            <div className="flex items-center gap-2">
              <input
                className="flex-1 min-w-0"
                value={sourcePath}
                readOnly
                placeholder="请选择 FLV 文件"
              />
              <button className="h-8 px-3 rounded-lg" onClick={handlePickSource}>
                选择文件
              </button>
            </div>
            <div className="flex items-center gap-2">
              <input
                className="flex-1 min-w-0"
                value={targetPath}
                readOnly
                placeholder="请选择输出 MP4 路径"
              />
              <button className="h-8 px-3 rounded-lg" onClick={handlePickTarget}>
                保存到
              </button>
            </div>
            <div className="flex flex-wrap items-center gap-3">
              <button
                className="h-8 px-3 rounded-lg"
                onClick={handleRemux}
                disabled={running}
              >
                {running ? "转封装中..." : "开始转封装"}
              </button>
              {message ? <span className="text-xs text-[var(--desc-color)]">{message}</span> : null}
            </div>
          </div>
        </div>

        <div className="panel p-4 space-y-1 text-xs text-[var(--desc-color)]">
          <div>1. 选择需要转封装的 FLV 文件。</div>
          <div>2. 选择 MP4 保存位置。</div>
          <div>3. 转封装会占用磁盘 IO，可能影响正在进行的录制。</div>
          <div>4. 如果录制文件存在问题，请先修复后再转封装。</div>
          <div>5. 转封装后无法再进行修复，请确认文件正常。</div>
        </div>
      </div>

      <div className="tab">
        {toolboxTabs.map((tab) => (
          <button
            key={tab.key}
            className={activeTab === tab.key ? "active" : ""}
            onClick={() => setActiveTab(tab.key)}
          >
            <span>{tab.label}</span>
            <label></label>
          </button>
        ))}
      </div>
    </div>
  );
}
