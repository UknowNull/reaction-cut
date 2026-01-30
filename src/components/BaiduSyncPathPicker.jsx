import { useEffect, useMemo, useState } from "react";
import { invokeCommand } from "../lib/tauri";

const replacePath = (path, fromPath, toPath) => {
  if (!path) {
    return path;
  }
  const from = fromPath.replace(/\/+$/, "");
  const to = toPath.replace(/\/+$/, "");
  const normalized = path.replace(/\/+$/, "");
  if (normalized === from) {
    return to;
  }
  if (normalized.startsWith(`${from}/`)) {
    return `${to}${normalized.slice(from.length)}`;
  }
  return path;
};

export default function BaiduSyncPathPicker({
  open,
  value,
  onConfirm,
  onClose,
  onChange,
}) {
  const [browserPath, setBrowserPath] = useState("/");
  const [folders, setFolders] = useState([]);
  const [browseLoading, setBrowseLoading] = useState(false);
  const [browseError, setBrowseError] = useState("");
  const [creating, setCreating] = useState(false);
  const [renamingPath, setRenamingPath] = useState("");
  const [renameValue, setRenameValue] = useState("新建文件夹");
  const [pinnedPath, setPinnedPath] = useState("");

  const orderedFolders = useMemo(() => {
    if (!pinnedPath) {
      return folders;
    }
    const index = folders.findIndex((folder) => folder.path === pinnedPath);
    if (index <= 0) {
      return folders;
    }
    const next = [...folders];
    const [item] = next.splice(index, 1);
    next.unshift(item);
    return next;
  }, [folders, pinnedPath]);

  const loadFolders = async (path) => {
    setBrowseError("");
    setBrowseLoading(true);
    try {
      const data = await invokeCommand("baidu_sync_remote_dirs", {
        request: { path },
      });
      setFolders(Array.isArray(data) ? data : []);
    } catch (error) {
      setBrowseError(error?.message || "读取目录失败");
      setFolders([]);
    } finally {
      setBrowseLoading(false);
    }
  };

  useEffect(() => {
    if (!open) {
      return;
    }
    const initialPath = String(value || "").trim() || "/";
    setBrowserPath(initialPath);
    setRenamingPath("");
    setRenameValue("新建文件夹");
    setPinnedPath("");
    loadFolders(initialPath);
  }, [open]);

  const handleClose = () => {
    if (browseLoading) {
      return;
    }
    onClose?.();
  };

  const handleConfirm = () => {
    onConfirm?.(browserPath);
    onClose?.();
  };

  const handleGoParent = () => {
    if (browserPath === "/") {
      return;
    }
    const segments = browserPath.split("/").filter(Boolean);
    segments.pop();
    const nextPath = segments.length ? `/${segments.join("/")}` : "/";
    setBrowserPath(nextPath);
    setRenamingPath("");
    loadFolders(nextPath);
  };

  const handleEnterFolder = (folder) => {
    if (!folder?.path) {
      return;
    }
    setBrowserPath(folder.path);
    setRenamingPath("");
    loadFolders(folder.path);
  };

  const handleCreateFolder = async () => {
    if (creating || browseLoading) {
      return;
    }
    setBrowseError("");
    setCreating(true);
    try {
      const data = await invokeCommand("baidu_sync_create_dir", {
        request: { parentPath: browserPath, name: "新建文件夹" },
      });
      const nextPath = data?.path || "";
      const nextName = data?.name || "新建文件夹";
      setRenamingPath(nextPath);
      setPinnedPath(nextPath);
      setRenameValue(nextName);
      await loadFolders(browserPath);
    } catch (error) {
      setBrowseError(error?.message || "创建目录失败");
    } finally {
      setCreating(false);
    }
  };

  const handleRenameCancel = () => {
    setRenamingPath("");
    setRenameValue("新建文件夹");
  };

  const handleRenameConfirm = async () => {
    if (!renamingPath) {
      return;
    }
    const trimmed = renameValue.trim();
    if (!trimmed) {
      setBrowseError("目录名称不能为空");
      return;
    }
    setBrowseError("");
    setBrowseLoading(true);
    try {
      const data = await invokeCommand("baidu_sync_rename_dir", {
        request: { fromPath: renamingPath, name: trimmed },
      });
      const nextPath = data?.path || renamingPath;
      if (pinnedPath && pinnedPath === renamingPath) {
        setPinnedPath(nextPath);
      }
      if (value) {
        const updatedValue = replacePath(value, renamingPath, nextPath);
        if (updatedValue !== value) {
          onChange?.(updatedValue);
        }
      }
      const nextBrowserPath = replacePath(browserPath, renamingPath, nextPath);
      if (nextBrowserPath !== browserPath) {
        setBrowserPath(nextBrowserPath);
      }
      setRenamingPath("");
      setRenameValue("新建文件夹");
      await loadFolders(nextBrowserPath);
    } catch (error) {
      setBrowseError(error?.message || "重命名失败");
    } finally {
      setBrowseLoading(false);
    }
  };

  const handleRenameKeyDown = (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      handleRenameConfirm();
    }
    if (event.key === "Escape") {
      event.preventDefault();
      handleRenameCancel();
    }
  };

  const handleStartRename = (folder) => {
    if (!folder?.path) {
      return;
    }
    setRenamingPath(folder.path);
    setRenameValue(folder.name || "新建文件夹");
  };

  if (!open) {
    return null;
  }

  return (
    <div className="fixed inset-0 z-[60] flex items-center justify-center bg-black/50">
      <div className="w-[520px] rounded-2xl bg-[var(--block-color)] p-5 text-sm text-[var(--content-color)] shadow-xl">
        <div className="text-base font-semibold">选择百度网盘目录</div>
        <div className="mt-2 flex items-center gap-2">
          <div className="flex-1 rounded-lg border border-black/10 bg-white/80 px-3 py-2 text-xs text-[var(--content-color)]">
            {browserPath}
          </div>
          <button
            className="rounded-full border border-black/10 bg-white px-3 py-1 text-xs font-semibold text-[var(--content-color)]"
            onClick={handleGoParent}
            disabled={browserPath === "/"}
          >
            上级
          </button>
          <button
            className="rounded-full border border-black/10 bg-white px-3 py-1 text-xs font-semibold text-[var(--content-color)]"
            onClick={handleCreateFolder}
            disabled={browseLoading || creating}
          >
            新建
          </button>
          <button
            className="rounded-full border border-black/10 bg-white px-3 py-1 text-xs font-semibold text-[var(--content-color)]"
            onClick={() => loadFolders(browserPath)}
            disabled={browseLoading}
          >
            刷新
          </button>
        </div>
        <div className="mt-3 max-h-64 overflow-auto rounded-xl border border-black/10 bg-white/80 p-2 text-xs text-[var(--content-color)]">
          {browseLoading ? (
            <div className="py-6 text-center text-[var(--desc-color)]">加载中...</div>
          ) : browseError ? (
            <div className="py-6 text-center text-amber-600">{browseError}</div>
          ) : orderedFolders.length === 0 ? (
            <div className="py-6 text-center text-[var(--desc-color)]">暂无目录</div>
          ) : (
            orderedFolders.map((folder) => {
              const isRenaming = folder.path === renamingPath;
              if (isRenaming) {
                return (
                  <div
                    key={folder.path}
                    className="flex items-center gap-2 rounded-lg px-2 py-2"
                  >
                    <span className="text-[10px] font-semibold text-[var(--muted)]">
                      DIR
                    </span>
                    <input
                      value={renameValue}
                      onChange={(event) => setRenameValue(event.target.value)}
                      onKeyDown={handleRenameKeyDown}
                      className="flex-1 rounded-lg border border-black/10 bg-white/80 px-2 py-1 text-xs focus:border-[var(--primary-color)] focus:outline-none"
                    />
                    <button
                      className="rounded-full border border-black/10 bg-white px-2 py-1 text-xs font-semibold text-[var(--content-color)]"
                      onClick={handleRenameConfirm}
                      disabled={browseLoading}
                    >
                      保存
                    </button>
                    <button
                      className="rounded-full border border-black/10 bg-white px-2 py-1 text-xs font-semibold text-[var(--content-color)]"
                      onClick={handleRenameCancel}
                      disabled={browseLoading}
                    >
                      取消
                    </button>
                  </div>
                );
              }
              return (
                <div
                  key={folder.path}
                  className="flex w-full items-center gap-2 rounded-lg px-2 py-2 hover:bg-black/5"
                >
                  <button
                    className="flex flex-1 items-center gap-2 text-left"
                    onClick={() => handleEnterFolder(folder)}
                  >
                    <span className="text-[10px] font-semibold text-[var(--muted)]">
                      DIR
                    </span>
                    <span className="text-sm">{folder.name}</span>
                  </button>
                  <button
                    className="inline-flex h-7 w-7 items-center justify-center rounded-full border border-black/10 bg-white text-[var(--content-color)] hover:bg-black/5"
                    onClick={() => handleStartRename(folder)}
                    title="重命名"
                    aria-label="重命名"
                  >
                    <svg viewBox="0 0 24 24" className="h-4 w-4" aria-hidden="true">
                      <path
                        d="M3 17.25V21h3.75l11-11-3.75-3.75-11 11zm2.92 2.83H5v-.92l8.5-8.5.92.92-8.5 8.5zM20.7 7.04a1 1 0 0 0 0-1.41l-2.33-2.33a1 1 0 0 0-1.41 0l-1.61 1.61 3.75 3.75 1.6-1.62z"
                        fill="currentColor"
                      />
                    </svg>
                  </button>
                </div>
              );
            })
          )}
        </div>
        <div className="mt-4 flex justify-end gap-2">
          <button className="h-9 rounded-lg px-4" onClick={handleClose}>
            取消
          </button>
          <button
            className="h-9 rounded-lg px-4"
            onClick={handleConfirm}
            disabled={browseLoading}
          >
            选择当前目录
          </button>
        </div>
      </div>
    </div>
  );
}
