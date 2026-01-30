import { useEffect, useMemo, useState } from "react";
import { invokeCommand } from "../lib/tauri";
import { formatDateTime } from "../lib/format";

const syncTabs = [
  { key: "pending", label: "待同步", statuses: ["PENDING", "PAUSED"] },
  { key: "uploading", label: "同步中", statuses: ["UPLOADING"] },
  { key: "completed", label: "已同步", statuses: ["SUCCESS"] },
  { key: "failed", label: "失败", statuses: ["FAILED", "CANCELLED"] },
];

const statusLabels = {
  PENDING: "待同步",
  PAUSED: "待同步",
  UPLOADING: "同步中",
  SUCCESS: "已同步",
  FAILED: "失败",
  CANCELLED: "失败",
};

const getStatusLabel = (status) => statusLabels[status] || status || "-";

const getStatusColor = (status) => {
  switch (status) {
    case "SUCCESS":
      return "#4caf50";
    case "FAILED":
    case "CANCELLED":
      return "#ff5252";
    case "UPLOADING":
      return "var(--primary-color)";
    case "PENDING":
    case "PAUSED":
    default:
      return "var(--split-color)";
  }
};

export default function SubmissionSyncSection() {
  const [tasks, setTasks] = useState([]);
  const [loading, setLoading] = useState(false);
  const [message, setMessage] = useState("");
  const [syncTab, setSyncTab] = useState("pending");

  const activeTabLabel =
    syncTabs.find((item) => item.key === syncTab)?.label || "同步任务";

  const filteredTasks = useMemo(() => {
    const current = syncTabs.find((item) => item.key === syncTab);
    if (!current) {
      return tasks;
    }
    return tasks.filter((task) => current.statuses.includes(task.status));
  }, [tasks, syncTab]);

  const loadTasks = async () => {
    setMessage("");
    setLoading(true);
    try {
      const list = await invokeCommand("baidu_sync_list", {
        request: {
          status: null,
          page: 1,
          pageSize: 200,
        },
      });
      setTasks(Array.isArray(list) ? list : []);
    } catch (error) {
      setMessage(error?.message || "加载同步任务失败");
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    loadTasks();
    const timer = setInterval(() => {
      loadTasks();
    }, 3000);
    return () => clearInterval(timer);
  }, []);

  const handleRetry = async (taskId) => {
    setMessage("");
    try {
      await invokeCommand("baidu_sync_retry", { taskId });
      await loadTasks();
    } catch (error) {
      setMessage(error?.message || "重新同步失败");
    }
  };

  const handlePause = async (taskId) => {
    setMessage("");
    try {
      await invokeCommand("baidu_sync_pause", { taskId });
      await loadTasks();
    } catch (error) {
      setMessage(error?.message || "暂停失败");
    }
  };

  const handleDelete = async (taskId) => {
    setMessage("");
    try {
      await invokeCommand("baidu_sync_delete", { taskId });
      await loadTasks();
    } catch (error) {
      setMessage(error?.message || "删除失败");
    }
  };

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h1 className="text-lg font-semibold text-[var(--content-color)]">视频同步</h1>
      </div>
      <div className="flex w-full h-full gap-3 min-h-0">
        <div className="flex-1 min-h-0">
          <div className="panel flex flex-col gap-2 p-3 min-h-0">
            <div className="flex items-center gap-2 px-1">
              <span className="text-sm font-semibold text-[var(--content-color)]">
                {activeTabLabel}（{filteredTasks.length}）
              </span>
              <button
                className="ml-auto h-8 px-3 rounded-lg"
                onClick={loadTasks}
                disabled={loading}
              >
                {loading ? "刷新中..." : "刷新"}
              </button>
            </div>
            <div className="flex flex-col gap-2 overflow-y-auto pr-1 min-h-0">
              {filteredTasks.length === 0 ? (
                <div className="desc px-2 py-6 text-center">暂无同步任务</div>
              ) : (
                filteredTasks.map((task) => {
                  const remotePath = `${task.remoteDir}/${task.remoteName}`.replace(
                    /\/+/g,
                    "/",
                  );
                  const progressValue = Math.min(100, Math.max(0, Number(task.progress || 0)));
                  const statusLabel = getStatusLabel(task.status);
                  const statusColor = getStatusColor(task.status);
                  const canPause = ["PENDING", "UPLOADING"].includes(task.status);
                  const canRetry = ["FAILED", "CANCELLED", "PAUSED", "SUCCESS"].includes(
                    task.status,
                  );
                  return (
                    <div
                      key={task.id}
                      className="flex flex-col gap-2 rounded-lg border-2 bg-[var(--block-color)] p-3 text-sm"
                      style={{ borderColor: statusColor }}
                    >
                      <div className="flex items-center gap-2 text-[var(--content-color)]">
                        <span className="truncate">{task.remoteName || "-"}</span>
                        <span className="rounded-full bg-black/5 px-2 py-0.5 text-xs">
                          {statusLabel}
                        </span>
                      </div>
                      <div className="flex flex-wrap gap-3 text-xs text-[var(--desc-color)]">
                        <span className="truncate">本地路径：{task.localPath || "-"}</span>
                        <span className="truncate">远程路径：{remotePath}</span>
                        <span>发起时间：{formatDateTime(task.createdAt)}</span>
                      </div>
                      <div className="flex items-center gap-3">
                        <div className="flex-1">
                          <div className="h-1.5 w-full rounded-full bg-[var(--solid-button-color)]">
                            <div
                              className="h-1.5 rounded-full"
                              style={{ width: `${progressValue}%`, backgroundColor: statusColor }}
                            />
                          </div>
                        </div>
                        <span className="w-12 text-xs text-[var(--desc-color)]">
                          {progressValue.toFixed(1)}%
                        </span>
                        {canRetry ? (
                          <button
                            className="h-8 px-3 rounded-lg"
                            onClick={() => handleRetry(task.id)}
                          >
                            重新同步
                          </button>
                        ) : null}
                        {canPause ? (
                          <button
                            className="h-8 px-3 rounded-lg"
                            onClick={() => handlePause(task.id)}
                          >
                            暂停
                          </button>
                        ) : null}
                        <button
                          className="h-8 px-3 rounded-lg"
                          onClick={() => handleDelete(task.id)}
                        >
                          删除
                        </button>
                      </div>
                    </div>
                  );
                })
              )}
            </div>
          </div>
        </div>
        <div className="tab">
          {syncTabs.map((tab) => (
            <button
              key={tab.key}
              className={syncTab === tab.key ? "active" : ""}
              onClick={() => setSyncTab(tab.key)}
            >
              <span>{tab.label}</span>
              <label />
            </button>
          ))}
        </div>
      </div>
      {message ? (
        <div className="rounded-lg border border-amber-200 bg-amber-50 px-3 py-2 text-sm text-amber-700">
          {message}
        </div>
      ) : null}
    </div>
  );
}
