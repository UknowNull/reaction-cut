import { useEffect, useMemo, useRef, useState } from "react";
import { invokeCommand } from "../lib/tauri";

const POLL_INTERVAL_MS = 3000;

const STATUS_LABELS = {
  idle: "待生成",
  pending: "等待扫码",
  scanned: "已扫码，请确认",
  expired: "二维码已过期",
  success: "登录成功",
};

export default function AuthSection({ onStatusChange }) {
  const [loading, setLoading] = useState(false);
  const [authStatus, setAuthStatus] = useState({ loggedIn: false });
  const [qrData, setQrData] = useState(null);
  const [qrStatus, setQrStatus] = useState("idle");
  const [message, setMessage] = useState("");
  const pollRef = useRef(null);

  const qrImageSrc = useMemo(() => {
    if (!qrData?.url) {
      return "";
    }
    const encoded = encodeURIComponent(qrData.url);
    return `https://api.qrserver.com/v1/create-qr-code/?size=220x220&data=${encoded}`;
  }, [qrData]);

  const stopPolling = () => {
    if (pollRef.current) {
      clearInterval(pollRef.current);
      pollRef.current = null;
    }
  };

  const refreshStatus = async () => {
    try {
      const data = await invokeCommand("auth_status");
      setAuthStatus(data || { loggedIn: false });
      if (onStatusChange) {
        onStatusChange(data || { loggedIn: false });
      }
    } catch (error) {
      setMessage(error.message);
    }
  };

  useEffect(() => {
    refreshStatus();
    return () => stopPolling();
  }, []);

  const handleGenerate = async () => {
    setLoading(true);
    setMessage("");
    try {
      const data = await invokeCommand("auth_qrcode_generate");
      if (!data?.url || !data?.qrcode_key) {
        throw new Error("二维码数据不正确");
      }
      setQrData({ url: data.url, key: data.qrcode_key });
      setQrStatus("pending");
      stopPolling();
      pollRef.current = setInterval(() => {
        pollStatus(data.qrcode_key);
      }, POLL_INTERVAL_MS);
    } catch (error) {
      setMessage(error.message);
    } finally {
      setLoading(false);
    }
  };

  const pollStatus = async (qrcodeKey) => {
    try {
      const data = await invokeCommand("auth_qrcode_poll", { qrcodeKey });
      const code = data?.code;
      if (code === 0) {
        setQrStatus("success");
        stopPolling();
        refreshStatus();
        return;
      }
      if (code === 86090) {
        setQrStatus("scanned");
        return;
      }
      if (code === 86038) {
        setQrStatus("expired");
        stopPolling();
        return;
      }
      setQrStatus("pending");
    } catch (error) {
      setMessage(error.message);
    }
  };

  const handleAutoLogin = async () => {
    setMessage("");
    try {
      await invokeCommand("auth_perform_qrcode_login");
      setMessage("登录流程已启动");
    } catch (error) {
      setMessage(error.message);
    }
  };

  const handleLogout = async () => {
    setMessage("");
    try {
      await invokeCommand("auth_logout");
      setAuthStatus({ loggedIn: false });
      setQrData(null);
      setQrStatus("idle");
      stopPolling();
    } catch (error) {
      setMessage(error.message);
    }
  };

  const displayName =
    authStatus?.userInfo?.data?.uname ||
    authStatus?.userInfo?.uname ||
    authStatus?.userInfo?.data?.username ||
    authStatus?.userInfo?.username ||
    "";

  return (
    <div className="space-y-6">
      <div className="rounded-2xl bg-[var(--surface)]/90 p-6 shadow-sm ring-1 ring-black/5">
        <div className="flex flex-wrap items-center justify-between gap-4">
          <div>
            <p className="text-sm uppercase tracking-[0.2em] text-[var(--muted)]">登录</p>
            <h2 className="text-2xl font-semibold text-[var(--ink)]">Bilibili 二维码登录</h2>
          </div>
          <div className="text-sm text-[var(--muted)]">
            {authStatus?.loggedIn ? "已登录" : "未登录"}
          </div>
        </div>
        {authStatus?.loggedIn && displayName ? (
          <div className="mt-3 text-sm text-[var(--ink)]">用户：{displayName}</div>
        ) : null}
        <div className="mt-5 flex flex-wrap gap-3">
          <button
            className="rounded-full bg-[var(--accent)] px-4 py-2 text-sm font-semibold text-white shadow-sm transition hover:brightness-110"
            onClick={handleGenerate}
            disabled={loading}
          >
            {loading ? "生成中..." : "生成二维码"}
          </button>
          {qrData?.url ? (
            <button
              className="rounded-full border border-black/10 bg-white px-4 py-2 text-sm font-semibold text-[var(--ink)] transition hover:border-black/20"
              onClick={handleGenerate}
              disabled={loading}
            >
              刷新二维码
            </button>
          ) : null}
          <button
            className="rounded-full border border-black/10 bg-white px-4 py-2 text-sm font-semibold text-[var(--ink)] transition hover:border-black/20"
            onClick={handleAutoLogin}
          >
            自动登录
          </button>
          <button
            className="rounded-full border border-black/10 bg-white px-4 py-2 text-sm font-semibold text-[var(--ink)] transition hover:border-black/20"
            onClick={refreshStatus}
          >
            刷新状态
          </button>
          <button
            className="rounded-full border border-black/10 bg-white px-4 py-2 text-sm font-semibold text-[var(--ink)] transition hover:border-black/20"
            onClick={handleLogout}
          >
            退出登录
          </button>
        </div>
      </div>

      <div className="grid gap-6 lg:grid-cols-[260px_1fr]">
        <div className="rounded-2xl bg-white/70 p-5 shadow-sm ring-1 ring-black/5">
          <div className="text-xs uppercase tracking-[0.2em] text-[var(--muted)]">二维码</div>
          {qrImageSrc ? (
            <div className="mt-4 flex items-center justify-center rounded-xl bg-white p-3">
              <img src={qrImageSrc} alt="二维码" className="h-48 w-48" />
            </div>
          ) : (
            <div className="mt-4 rounded-xl border border-dashed border-black/10 p-6 text-sm text-[var(--muted)]">
              请先生成二维码开始登录。
            </div>
          )}
          {qrData?.url ? (
            <div className="mt-3 break-all text-xs text-[var(--muted)]">{qrData.url}</div>
          ) : null}
        </div>

        <div className="rounded-2xl bg-[var(--surface)]/90 p-6 shadow-sm ring-1 ring-black/5">
          <div className="text-xs uppercase tracking-[0.2em] text-[var(--muted)]">状态</div>
          <div className="mt-3 text-lg font-semibold text-[var(--ink)]">
            {STATUS_LABELS[qrStatus] || "待生成"}
          </div>
          <div className="mt-4 text-sm text-[var(--muted)]">
            请保持二维码可见，并使用手机端扫码确认。
          </div>
          {message ? (
            <div className="mt-4 rounded-lg border border-red-200 bg-red-50 px-3 py-2 text-sm text-red-700">
              {message}
            </div>
          ) : null}
        </div>
      </div>
    </div>
  );
}
