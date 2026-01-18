import { useEffect, useState } from "react";
import { invokeCommand } from "../lib/tauri";

const createClip = (file, index) => ({
  fileName: file.name || "未知文件",
  filePath: file.path || file.name || "",
  startTime: "00:00:00",
  endTime: "00:00:00",
  sequence: index + 1,
});

export default function ProcessSection() {
  const [taskName, setTaskName] = useState("");
  const [clips, setClips] = useState([]);
  const [processing, setProcessing] = useState(false);
  const [taskId, setTaskId] = useState(null);
  const [taskInfo, setTaskInfo] = useState(null);
  const [message, setMessage] = useState("");

  const updateClip = (index, field, value) => {
    setClips((prev) =>
      prev.map((clip, idx) => (idx === index ? { ...clip, [field]: value } : clip)),
    );
  };

  const handleFileChange = (event) => {
    const files = Array.from(event.target.files || []);
    if (files.length === 0) {
      return;
    }
    setClips((prev) => {
      const startIndex = prev.length;
      const next = files.map((file, index) => createClip(file, startIndex + index));
      return [...prev, ...next];
    });
    event.target.value = "";
  };

  const removeClip = (index) => {
    setClips((prev) =>
      prev
        .filter((_, idx) => idx !== index)
        .map((clip, idx) => ({ ...clip, sequence: idx + 1 })),
    );
  };

  useEffect(() => {
    if (!taskId) {
      return undefined;
    }
    let active = true;
    let timer;
    const fetchStatus = async () => {
      try {
        const data = await invokeCommand("process_status", { task_id: taskId });
        if (!active) {
          return;
        }
        setTaskInfo(data);
        if (data?.status === 2 || data?.status === 3) {
          clearInterval(timer);
        }
      } catch (error) {
        if (active) {
          setMessage(error.message);
        }
      }
    };
    fetchStatus();
    timer = setInterval(fetchStatus, 3000);
    return () => {
      active = false;
      if (timer) {
        clearInterval(timer);
      }
    };
  }, [taskId]);

  const handleCreate = async () => {
    setMessage("");
    if (!taskName.trim()) {
      setMessage("请输入任务名称");
      return;
    }
    const validClips = clips.filter((clip) => clip.filePath.trim());
    if (validClips.length === 0) {
      setMessage("请至少添加一个片段");
      return;
    }
    setProcessing(true);
    try {
      const payload = {
        request: {
          taskName,
          clips: validClips.map((clip, index) => ({
            filePath: clip.filePath,
            fileName: clip.fileName || null,
            startTime: clip.startTime || "00:00:00",
            endTime: clip.endTime || "00:00:00",
            sequence: clip.sequence || index + 1,
          })),
        },
      };
      const taskId = await invokeCommand("process_create", payload);
      setTaskId(taskId);
      setTaskInfo(null);
    } catch (error) {
      setMessage(error.message);
    } finally {
      setProcessing(false);
    }
  };

  const formatStatus = (status) => {
    if (status === 0) return "待处理";
    if (status === 1) return "处理中";
    if (status === 2) return "处理完成";
    if (status === 3) return "处理失败";
    return "未知";
  };

  const formatUploadStatus = (status) => {
    if (status === 0) return "未投稿";
    if (status === 1) return "投稿中";
    if (status === 2) return "投稿成功";
    if (status === 3) return "投稿失败";
    return "未知";
  };

  const statusTone = (status) => {
    if (status === 2) return "bg-emerald-500/10 text-emerald-600";
    if (status === 3) return "bg-rose-500/10 text-rose-600";
    if (status === 1) return "bg-amber-500/10 text-amber-600";
    return "bg-slate-500/10 text-slate-600";
  };

  return (
    <div className="space-y-6">
      <div className="rounded-2xl bg-[var(--surface)]/90 p-6 shadow-sm ring-1 ring-black/5">
        <div>
          <p className="text-sm uppercase tracking-[0.2em] text-[var(--muted)]">视频处理</p>
          <h2 className="text-2xl font-semibold text-[var(--ink)]">视频处理</h2>
        </div>
        <div className="mt-4 space-y-3">
          <input
            value={taskName}
            onChange={(event) => setTaskName(event.target.value)}
            placeholder="任务名称"
            className="w-full rounded-xl border border-black/10 bg-white/80 px-3 py-2 text-sm text-[var(--ink)] focus:border-[var(--accent)] focus:outline-none"
          />
          <div className="flex flex-wrap items-center gap-3">
            <label className="rounded-full bg-[var(--accent)] px-4 py-2 text-sm font-semibold text-white shadow-sm transition hover:brightness-110">
              选择文件
              <input
                type="file"
                multiple
                className="hidden"
                onChange={handleFileChange}
                accept="video/*"
              />
            </label>
            <span className="text-xs text-[var(--muted)]">请选择要处理的视频文件</span>
          </div>
        </div>
      </div>

      {clips.length > 0 ? (
        <div className="rounded-2xl bg-white/80 p-6 shadow-sm ring-1 ring-black/5">
          <div className="text-xs uppercase tracking-[0.2em] text-[var(--muted)]">
            视频片段设置
          </div>
          <div className="mt-3 overflow-hidden rounded-xl border border-black/5">
            <table className="w-full text-left text-sm">
              <thead className="bg-black/5 text-xs uppercase tracking-[0.2em] text-[var(--muted)]">
                <tr>
                  <th className="px-4 py-2">文件名</th>
                  <th className="px-4 py-2">序号</th>
                  <th className="px-4 py-2">开始时间</th>
                  <th className="px-4 py-2">结束时间</th>
                  <th className="px-4 py-2">操作</th>
                </tr>
              </thead>
              <tbody>
                {clips.map((clip, index) => (
                  <tr key={`clip-${index}`} className="border-t border-black/5">
                    <td className="px-4 py-2 text-[var(--ink)]">
                      <div className="font-medium">{clip.fileName || "未知文件"}</div>
                      <div className="text-xs text-[var(--muted)]">{clip.filePath}</div>
                    </td>
                    <td className="px-4 py-2">
                      <input
                        type="number"
                        min={1}
                        value={clip.sequence}
                        onChange={(event) =>
                          updateClip(index, "sequence", Number(event.target.value))
                        }
                        className="w-20 rounded-lg border border-black/10 bg-white/80 px-2 py-1 text-sm focus:border-[var(--accent)] focus:outline-none"
                      />
                    </td>
                    <td className="px-4 py-2">
                      <input
                        value={clip.startTime}
                        onChange={(event) => updateClip(index, "startTime", event.target.value)}
                        placeholder="00:00:00"
                        className="w-full rounded-lg border border-black/10 bg-white/80 px-2 py-1 text-sm focus:border-[var(--accent)] focus:outline-none"
                      />
                    </td>
                    <td className="px-4 py-2">
                      <input
                        value={clip.endTime}
                        onChange={(event) => updateClip(index, "endTime", event.target.value)}
                        placeholder="00:00:00"
                        className="w-full rounded-lg border border-black/10 bg-white/80 px-2 py-1 text-sm focus:border-[var(--accent)] focus:outline-none"
                      />
                    </td>
                    <td className="px-4 py-2">
                      <button
                        type="button"
                        onClick={() => removeClip(index)}
                        className="rounded-full border border-black/10 bg-white px-3 py-1 text-xs font-semibold text-[var(--ink)] transition hover:border-black/20"
                      >
                        删除
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <div className="mt-4 text-center">
            <button
              className="rounded-full bg-[var(--accent)] px-4 py-2 text-sm font-semibold text-white shadow-sm transition hover:brightness-110"
              onClick={handleCreate}
              disabled={processing}
            >
              {processing ? "提交中..." : "提交处理任务"}
            </button>
          </div>
          {message ? (
            <div className="rounded-lg border border-amber-200 bg-amber-50 px-3 py-2 text-sm text-amber-700">
              {message}
            </div>
          ) : null}
        </div>
      ) : null}

      {taskId ? (
        <div className="rounded-2xl bg-white/80 p-6 shadow-sm ring-1 ring-black/5">
          <div className="text-xs uppercase tracking-[0.2em] text-[var(--muted)]">
            任务状态
          </div>
          <div className="mt-3 grid gap-2 text-sm text-[var(--ink)]">
            <div>任务ID：{taskId}</div>
            <div>任务名称：{taskInfo?.taskName || "获取中..."}</div>
            <div>
              状态：
              <span
                className={`ml-2 rounded-full px-2 py-0.5 text-xs font-semibold ${statusTone(
                  taskInfo?.status,
                )}`}
              >
                {formatStatus(taskInfo?.status)}
              </span>
            </div>
            <div className="mt-1 w-full">
              <div className="h-2 w-full rounded-full bg-black/5">
                <div
                  className="h-2 rounded-full bg-[var(--accent)]"
                  style={{ width: `${Math.min(100, taskInfo?.progress || 0)}%` }}
                />
              </div>
              <div className="mt-1 text-xs text-[var(--muted)]">
                进度：{taskInfo?.progress || 0}%
              </div>
            </div>
            <div>
              投稿状态：
              <span
                className={`ml-2 rounded-full px-2 py-0.5 text-xs font-semibold ${statusTone(
                  taskInfo?.uploadStatus,
                )}`}
              >
                {formatUploadStatus(taskInfo?.uploadStatus)}
              </span>
            </div>
            {taskInfo?.bilibiliUrl ? (
              <div>
                视频链接：
                <a
                  href={taskInfo.bilibiliUrl}
                  target="_blank"
                  rel="noreferrer"
                  className="ml-2 text-[var(--accent)] underline"
                >
                  {taskInfo.bilibiliUrl}
                </a>
              </div>
            ) : null}
          </div>
        </div>
      ) : null}
    </div>
  );
}
