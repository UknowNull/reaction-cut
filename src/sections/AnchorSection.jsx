import { useEffect, useState } from "react";
import { invokeCommand } from "../lib/tauri";
import { formatDateTime } from "../lib/format";

export default function AnchorSection() {
  const [newAnchorUid, setNewAnchorUid] = useState("");
  const [anchors, setAnchors] = useState([]);
  const [anchorAvatars, setAnchorAvatars] = useState({});
  const [message, setMessage] = useState("");
  const [loading, setLoading] = useState(false);

  const logClient = async (text) => {
    try {
      await invokeCommand("auth_client_log", { message: text });
    } catch (error) {
      // ignore log errors
    }
  };

  const loadAnchors = async () => {
    setMessage("");
    setLoading(true);
    try {
      await logClient("anchor_list:load_start");
      const data = await invokeCommand("anchor_check");
      setAnchors(data || []);
      await logClient(`anchor_list:load_ok:${Array.isArray(data) ? data.length : 0}`);
    } catch (error) {
      await logClient(`anchor_list:load_error:${error?.message || "unknown"}`);
      setMessage(error.message);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    loadAnchors();
    const timer = setInterval(loadAnchors, 30000);
    return () => clearInterval(timer);
  }, []);

  useEffect(() => {
    let active = true;
    const loadAvatars = async () => {
      const updates = {};
      for (const anchor of anchors) {
        if (!anchor?.avatarUrl) {
          continue;
        }
        if (anchorAvatars[anchor.uid]) {
          continue;
        }
        try {
          const data = await invokeCommand("video_proxy_image", { url: anchor.avatarUrl });
          if (data) {
            updates[anchor.uid] = data;
          }
        } catch (error) {
          // ignore avatar proxy errors
        }
      }
      if (active && Object.keys(updates).length > 0) {
        setAnchorAvatars((prev) => ({ ...prev, ...updates }));
      }
    };
    loadAvatars();
    return () => {
      active = false;
    };
  }, [anchors]);

  const handleSubscribe = async () => {
    const uid = extractUidFromInput(newAnchorUid);
    await logClient(`anchor_subscribe:click raw=${newAnchorUid || ""}`);
    if (!uid) {
      await logClient("anchor_subscribe:invalid_input");
      setMessage("请输入房间号或链接");
      return;
    }
    setLoading(true);
    setMessage("");
    try {
      await logClient(`anchor_subscribe:invoke_start uid=${uid}`);
      await invokeCommand("anchor_subscribe", { payload: { uids: [uid] } });
      await logClient("anchor_subscribe:invoke_ok");
      await loadAnchors();
      setNewAnchorUid("");
    } catch (error) {
      await logClient(`anchor_subscribe:invoke_error:${error?.message || "unknown"}`);
      setMessage(error.message);
    } finally {
      setLoading(false);
    }
  };

  const handleUnsubscribe = async (anchor) => {
    setMessage("");
    try {
      await invokeCommand("anchor_unsubscribe", { uid: anchor.uid });
      await loadAnchors();
    } catch (error) {
      setMessage(error.message);
    }
  };

  const handleStartRecord = async (anchor) => {
    setMessage("");
    try {
      await invokeCommand("live_record_start", { roomId: anchor.uid });
      await loadAnchors();
    } catch (error) {
      setMessage(error.message);
    }
  };

  const handleStopRecord = async (anchor) => {
    setMessage("");
    try {
      await invokeCommand("live_record_stop", { roomId: anchor.uid });
      await loadAnchors();
    } catch (error) {
      setMessage(error.message);
    }
  };

  const handleAutoRecordToggle = async (anchor) => {
    setMessage("");
    try {
      await invokeCommand("live_room_auto_record_update", {
        roomId: anchor.uid,
        autoRecord: !anchor.autoRecord,
      });
      await loadAnchors();
    } catch (error) {
      setMessage(error.message);
    }
  };

  const extractUidFromInput = (input) => {
    if (!input) {
      return "";
    }
    const trimmed = input.trim();
    if (/^\d+$/.test(trimmed)) {
      return trimmed;
    }
    const match = trimmed.match(/live\.bilibili\.com\/(\d+)/);
    if (match && match[1]) {
      return match[1];
    }
    return trimmed;
  };

  const statusLabel = (status) => {
    if (status === 1) {
      return "直播中";
    }
    if (status === 2) {
      return "轮播中";
    }
    return "未直播";
  };

  return (
    <div className="space-y-6">
      <div className="rounded-2xl bg-[var(--surface)]/90 p-6 shadow-sm ring-1 ring-black/5">
        <div className="flex flex-wrap items-center justify-between gap-4">
          <div>
            <p className="text-sm uppercase tracking-[0.2em] text-[var(--muted)]">主播订阅</p>
            <h2 className="text-2xl font-semibold text-[var(--ink)]">主播订阅管理</h2>
          </div>
          <div className="flex gap-2">
            <button
              className="rounded-full border border-black/10 bg-white px-4 py-2 text-sm font-semibold text-[var(--ink)] transition hover:border-black/20"
              onClick={loadAnchors}
              disabled={loading}
            >
              刷新
            </button>
          </div>
        </div>
        <div className="mt-4 text-sm text-[var(--muted)]">
          从这里开始订阅直播间，状态会在卡片上实时更新。
        </div>
        {message ? (
          <div className="mt-4 rounded-lg border border-amber-200 bg-amber-50 px-3 py-2 text-sm text-amber-700">
            {message}
          </div>
        ) : null}
      </div>

      <div className="rounded-2xl bg-white/80 p-6 shadow-sm ring-1 ring-black/5">
        <div className="text-xs uppercase tracking-[0.2em] text-[var(--muted)]">直播间列表</div>
        <div className="mt-4 grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
          <div className="rounded-2xl border border-dashed border-black/15 bg-white/70 p-4">
            <div className="flex items-center gap-2 text-sm font-semibold text-[var(--ink)]">
              <span className="text-lg">＋</span>
              新增直播间
            </div>
            <input
              value={newAnchorUid}
              onChange={(event) => setNewAnchorUid(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  handleSubscribe();
                }
              }}
              placeholder="房间号或链接"
              className="mt-3 w-full rounded-xl border border-black/10 bg-white/80 px-3 py-2 text-sm text-[var(--ink)] focus:border-[var(--accent)] focus:outline-none"
            />
            <button
              className="mt-3 w-full rounded-xl bg-[var(--accent)] px-4 py-2 text-sm font-semibold text-white shadow-sm transition hover:brightness-110"
              onClick={handleSubscribe}
              disabled={loading}
            >
              订阅
            </button>
            <div className="mt-2 text-xs text-[var(--muted)]">支持房间号或直播间链接</div>
          </div>
          {anchors.map((anchor) => (
            <div key={anchor.id} className="rounded-2xl border border-black/5 bg-white/90 p-4">
              <div className="flex items-start justify-between gap-3">
                <div className="flex items-center gap-3">
                  <div className="h-12 w-12 overflow-hidden rounded-full bg-black/5">
                    {anchorAvatars[anchor.uid] ? (
                      <img
                        src={anchorAvatars[anchor.uid]}
                        alt={anchor.nickname || "主播"}
                        className="h-full w-full object-cover"
                      />
                    ) : (
                      <div className="flex h-full w-full items-center justify-center text-xs text-[var(--muted)]">
                        头像
                      </div>
                    )}
                  </div>
                  <div>
                    <div className="text-sm font-semibold text-[var(--ink)]">
                      {anchor.nickname || "未知主播"}
                    </div>
                    <div className="text-xs text-[var(--muted)]">房间号：{anchor.uid}</div>
                  </div>
                </div>
                <div className="flex flex-col items-end gap-2">
                  <span
                    className={`rounded-full px-2 py-0.5 text-xs font-semibold ${
                      anchor.liveStatus === 1
                        ? "bg-emerald-500/10 text-emerald-600"
                        : "bg-slate-500/10 text-slate-600"
                    }`}
                  >
                    {statusLabel(anchor.liveStatus)}
                  </span>
                  {anchor.recordingStatus ? (
                    <span className="rounded-full bg-amber-500/10 px-2 py-0.5 text-xs font-semibold text-amber-600">
                      录制中
                    </span>
                  ) : anchor.autoRecord ? (
                    <span className="rounded-full bg-orange-500/10 px-2 py-0.5 text-xs font-semibold text-orange-600">
                      监控中
                    </span>
                  ) : null}
                </div>
              </div>
              {anchor.liveStatus === 1 ? (
                <div className="mt-3 text-sm text-[var(--ink)]">
                  <div className="font-semibold">{anchor.liveTitle || "直播标题"}</div>
                  <div className="text-xs text-[var(--muted)]">
                    {anchor.category || "未知分区"}
                  </div>
                </div>
              ) : (
                <div className="mt-3 text-xs text-[var(--muted)]">当前未开播</div>
              )}
              <div className="mt-3 flex flex-wrap items-center gap-2 text-xs text-[var(--muted)]">
                <span>自动录制：{anchor.autoRecord ? "已开启" : "已关闭"}</span>
                <button
                  className="rounded-full border border-black/10 bg-white px-2 py-1 text-xs font-semibold text-[var(--ink)]"
                  onClick={() => handleAutoRecordToggle(anchor)}
                >
                  {anchor.autoRecord ? "关闭" : "开启"}
                </button>
                <span>上次检查：{formatDateTime(anchor.lastCheckTime)}</span>
              </div>
              <div className="mt-3 flex flex-wrap gap-2">
                {anchor.liveStatus === 1 ? (
                  <button
                    className="rounded-full border border-black/10 bg-white px-3 py-1.5 text-xs font-semibold text-[var(--ink)]"
                    onClick={() => handleStopRecord(anchor)}
                  >
                    停止录制
                  </button>
                ) : (
                  <button
                    className="rounded-full border border-black/10 bg-white px-3 py-1.5 text-xs font-semibold text-[var(--ink)]"
                    onClick={() => handleStartRecord(anchor)}
                  >
                    开始录制
                  </button>
                )}
                <button
                  className="rounded-full border border-black/10 bg-white px-3 py-1.5 text-xs font-semibold text-[var(--ink)]"
                  onClick={() => handleUnsubscribe(anchor)}
                >
                  取消订阅
                </button>
              </div>
              {anchor.recordingFile ? (
                <div className="mt-2 text-xs text-[var(--muted)]">
                  当前文件：{anchor.recordingFile}
                </div>
              ) : null}
              {anchor.recordingStartTime ? (
                <div className="mt-1 text-xs text-[var(--muted)]">
                  开始时间：{formatDateTime(anchor.recordingStartTime)}
                </div>
              ) : null}
            </div>
          ))}
        </div>
        {anchors.length === 0 ? (
          <div className="mt-4 text-sm text-[var(--muted)]">暂无订阅记录。</div>
        ) : null}
      </div>
    </div>
  );
}
