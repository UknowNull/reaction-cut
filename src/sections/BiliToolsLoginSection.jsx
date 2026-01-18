import { useEffect, useMemo, useRef, useState } from "react";
import JSEncrypt from "jsencrypt";
import { invokeCommand } from "../lib/tauri";

const DEFAULT_AVATAR = "https://i0.hdslb.com/bfs/face/member/noface.jpg";

const defaultUser = {
  avatar: DEFAULT_AVATAR,
  name: "Bilibili 用户",
  desc: "暂无签名",
  stat: {
    following: 0,
    follower: 0,
    dynamic: 0,
    coins: 0,
  },
};

const statusTextMap = {
  "-2": "加载中...",
  "86101": "等待扫描...",
  "86090": "扫码成功，请在手机上确认",
  "86038": "二维码已过期",
};

const requestJson = async (url, options = {}) => {
  const response = await fetch(url, options);
  let body = null;
  try {
    body = await response.json();
  } catch (error) {
    throw new Error("请求解析失败");
  }
  if (typeof body?.code === "number" && body.code !== 0) {
    throw new Error(body.message || "请求失败");
  }
  return body?.data ?? body;
};

const getCaptchaParams = async () =>
  requestJson("https://passport.bilibili.com/x/passport-login/captcha?source=main-fe-header");

const runCaptcha = (gt, challenge) =>
  new Promise((resolve, reject) => {
    if (!window.initGeetest) {
      reject(new Error("验证码组件未加载"));
      return;
    }
    window.initGeetest(
      {
        gt,
        challenge,
        offline: false,
        new_captcha: true,
        product: "bind",
        width: "300px",
        lang: "zh-cn",
        https: true,
      },
      (captchaObj) => {
        captchaObj
          .onReady(() => {
            captchaObj.verify();
          })
          .onSuccess(() => {
            const result = captchaObj.getValidate();
            resolve({
              challenge,
              validate: result.geetest_validate,
              seccode: result.geetest_seccode,
            });
          })
          .onError((error) => {
            reject(new Error(error?.msg || "验证码校验失败"));
          })
          .onClose(() => {
            reject(new Error("验证码已取消"));
          });
      },
    );
  });

export default function BiliToolsLoginSection({
  onStatusChange,
  embedded = false,
  initialStatus = null,
}) {
  const [isLogin, setIsLogin] = useState(Boolean(initialStatus?.loggedIn));
  const [scanStatus, setScanStatus] = useState(-1);
  const [qrData, setQrData] = useState(null);
  const [user, setUser] = useState(defaultUser);
  const [message, setMessage] = useState("");
  const [loginTab, setLoginTab] = useState("sms");
  const [accountForm, setAccountForm] = useState({ username: "", password: "" });
  const [smsForm, setSmsForm] = useState({ cid: "", tel: "", code: "" });
  const [countries, setCountries] = useState([]);
  const [captchaKey, setCaptchaKey] = useState("");
  const [sendingSms, setSendingSms] = useState(false);
  const [loginLoading, setLoginLoading] = useState(false);
  const [avatarPreview, setAvatarPreview] = useState("");
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

  const parseUserInfo = (status) => {
    const raw = status?.userInfo || {};
    const level1 = raw?.data || raw;
    const level2 = level1?.data || level1;
    const name =
      level2?.uname ||
      level2?.username ||
      level2?.name ||
      level1?.uname ||
      level1?.username ||
      level1?.name ||
      "Bilibili 用户";
    const avatar =
      level2?.avatar ||
      level2?.face ||
      level1?.avatar ||
      level1?.face ||
      DEFAULT_AVATAR;
    const stat = level2?.stat || level1?.stat || defaultUser.stat;
    return {
      ...defaultUser,
      name,
      avatar,
      desc: level2?.sign || level2?.desc || defaultUser.desc,
      stat: {
        following: stat?.following ?? defaultUser.stat.following,
        follower: stat?.follower ?? defaultUser.stat.follower,
        dynamic: stat?.dynamic ?? defaultUser.stat.dynamic,
        coins:
          stat?.coins ??
          level2?.coins ??
          level1?.coins ??
          defaultUser.stat.coins,
      },
    };
  };

  const refreshStatus = async () => {
    try {
      const data = await invokeCommand("auth_status");
      const loggedIn = Boolean(data?.loggedIn);
      setIsLogin(loggedIn);
      if (loggedIn) {
        setUser(parseUserInfo(data));
        setMessage("");
      }
      if (onStatusChange) {
        onStatusChange(data || { loggedIn: false });
      }
      return loggedIn;
    } catch (error) {
      setMessage(error.message);
      return false;
    }
  };

  const pollStatus = async (qrcodeKey) => {
    try {
      await invokeCommand("auth_client_log", { message: `poll_tick:${String(qrcodeKey).length}` });
      const data = await invokeCommand("auth_qrcode_poll", { qrcodeKey });
      const code = data?.code ?? 86101;
      setScanStatus(code);
      if (code === 0) {
        stopPolling();
        await refreshStatus();
        return;
      }
      if (code === 86038) {
        stopPolling();
      }
    } catch (error) {
      const raw = typeof error === "string" ? error : String(error || "");
      const message = error?.message || raw || "轮询失败";
      setMessage(message);
      try {
        await invokeCommand("auth_client_log", {
          message: `poll_error:${message}|raw:${raw}`,
        });
      } catch (_) {
        // ignore
      }
    }
  };

  const initScan = async () => {
    stopPolling();
    setMessage("");
    setScanStatus(-2);
    setQrData(null);
    try {
      const data = await invokeCommand("auth_qrcode_generate");
      if (!data?.url || !data?.qrcode_key) {
        throw new Error("二维码生成失败");
      }
      setQrData({ url: data.url, key: data.qrcode_key });
      setScanStatus(86101);
      pollRef.current = setInterval(() => {
        pollStatus(data.qrcode_key);
      }, 3000);
      await invokeCommand("auth_client_log", { message: `poll_start:${data.qrcode_key.length}` });
    } catch (error) {
      setMessage(error.message);
    }
  };

  const loadAvatar = async (url) => {
    if (!url) {
      setAvatarPreview("");
      return;
    }
    try {
      const data = await invokeCommand("video_proxy_image", { url });
      setAvatarPreview(data || "");
      await invokeCommand("auth_client_log", {
        message: `avatar_proxy_ok:${String(url).length}:${String(data || "").length}`,
      });
    } catch (error) {
      const message = error?.message || String(error || "");
      await invokeCommand("auth_client_log", {
        message: `avatar_proxy_fail:${String(url).length}:${message}`,
      });
    }
  };

  const loadSmsMeta = async () => {
    try {
      const zone = await requestJson("https://api.bilibili.com/x/web-interface/zone");
      const code = zone?.country_code ? String(zone.country_code) : "";
      setSmsForm((prev) => ({ ...prev, cid: code }));
      const countryList = await requestJson("https://passport.bilibili.com/web/generic/country/list");
      const list = [...(countryList?.common || []), ...(countryList?.others || [])].map((item) => ({
        id: String(item.country_id),
        name: `+${item.country_id}`,
      }));
      setCountries(list);
    } catch (error) {
      setMessage(error.message);
    }
  };

  const handleLogout = async () => {
    setMessage("");
    try {
      await invokeCommand("auth_logout");
      setIsLogin(false);
      setUser(defaultUser);
      if (onStatusChange) {
        onStatusChange({ loggedIn: false });
      }
      initScan();
    } catch (error) {
      setMessage(error.message);
    }
  };

  const handleSendSms = async () => {
    if (!smsForm.tel) {
      setMessage("请输入手机号");
      return;
    }
    if (!smsForm.cid) {
      setMessage("请选择区号");
      return;
    }
    setSendingSms(true);
    setMessage("");
    try {
      const { token, geetest } = await getCaptchaParams();
      const captcha = await runCaptcha(geetest?.gt, geetest?.challenge);
      const params = new URLSearchParams({
        cid: smsForm.cid,
        tel: smsForm.tel,
        token,
        source: "main-fe-header",
        challenge: captcha.challenge,
        validate: captcha.validate,
        seccode: captcha.seccode,
      });
      const response = await fetch(
        "https://passport.bilibili.com/x/passport-login/web/sms/send",
        {
          method: "POST",
          headers: { "Content-Type": "application/x-www-form-urlencoded" },
          body: params.toString(),
        },
      );
      const body = await response.json();
      if (body.code !== 0) {
        throw new Error(body.message || "发送失败");
      }
      setCaptchaKey(body?.data?.captcha_key || "");
      setMessage("短信验证码已发送");
    } catch (error) {
      setMessage(error.message);
    } finally {
      setSendingSms(false);
    }
  };

  const handleSmsLogin = async () => {
    if (!smsForm.tel || !smsForm.code) {
      setMessage("请填写手机号和验证码");
      return;
    }
    if (!captchaKey) {
      setMessage("请先获取短信验证码");
      return;
    }
    setLoginLoading(true);
    setMessage("");
    try {
      await invokeCommand("auth_sms_login", {
        cid: Number(smsForm.cid),
        tel: smsForm.tel,
        code: smsForm.code,
        captcha_key: captchaKey,
      });
      stopPolling();
      await refreshStatus();
    } catch (error) {
      setMessage(error.message);
    } finally {
      setLoginLoading(false);
    }
  };

  const handlePwdLogin = async () => {
    if (!accountForm.username || !accountForm.password) {
      setMessage("请输入账号和密码");
      return;
    }
    setLoginLoading(true);
    setMessage("");
    try {
      const { token, geetest } = await getCaptchaParams();
      const captcha = await runCaptcha(geetest?.gt, geetest?.challenge);
      const keyData = await requestJson(
        "https://passport.bilibili.com/x/passport-login/web/key",
      );
      const encrypt = new JSEncrypt();
      encrypt.setPublicKey(keyData.key);
      const encodedPwd = encrypt.encrypt(`${keyData.hash}${accountForm.password}`);
      if (!encodedPwd) {
        throw new Error("密码加密失败");
      }
      await invokeCommand("auth_pwd_login", {
        username: accountForm.username,
        encoded_pwd: encodedPwd,
        token,
        challenge: captcha.challenge,
        validate: captcha.validate,
        seccode: captcha.seccode,
      });
      stopPolling();
      await refreshStatus();
    } catch (error) {
      setMessage(error.message);
    } finally {
      setLoginLoading(false);
    }
  };

  useEffect(() => {
    if (initialStatus?.loggedIn) {
      setIsLogin(true);
      setUser(parseUserInfo(initialStatus));
      setMessage("");
    }
  }, [initialStatus]);

  useEffect(() => {
    const init = async () => {
      if (initialStatus?.loggedIn) {
        await refreshStatus();
        return;
      }
      const loggedIn = await refreshStatus();
      if (!loggedIn) {
        initScan();
        loadSmsMeta();
      }
    };
    init();
    return () => stopPolling();
  }, [initialStatus?.loggedIn]);

  useEffect(() => {
    if (isLogin) {
      loadAvatar(user.avatar);
    } else {
      setAvatarPreview("");
    }
  }, [isLogin, user.avatar]);

  useEffect(() => {
    if (!isLogin) {
      return;
    }
    invokeCommand("auth_client_log", {
      message: `avatar_preview_update:${String(avatarPreview).length}`,
    }).catch(() => {});
    if (!avatarPreview) {
      invokeCommand("auth_client_log", {
        message: "avatar_fallback_render",
      }).catch(() => {});
    }
  }, [avatarPreview, isLogin]);

  return (
    <div className={embedded ? "" : "space-y-4"}>
      <div className="flex items-center justify-between">
        <h1 className="text-lg font-semibold text-[var(--content-color)]">账号登录</h1>
        {isLogin ? (
          <button className="h-8 px-3 rounded-lg" onClick={handleLogout}>
            退出登录
          </button>
        ) : null}
      </div>

      {!isLogin ? (
        <div className="panel flex flex-col gap-6 p-4 lg:flex-row">
          <div className="flex flex-col items-center gap-3 border-b border-[var(--split-color)] pb-4 lg:w-72 lg:border-b-0 lg:border-r lg:pb-0 lg:pr-4">
            <div className="text-base font-semibold text-[var(--content-color)]">扫码登录</div>
            <div className="relative flex h-48 w-48 items-center justify-center rounded-lg bg-white">
              {qrImageSrc ? (
                <img src={qrImageSrc} alt="二维码" className="h-40 w-40" />
              ) : (
                <div className="text-sm text-[var(--desc-color)]">二维码加载中...</div>
              )}
              {scanStatus === -2 ? (
                <div className="absolute inset-0 flex flex-col items-center justify-center gap-2 bg-white/80 text-sm text-[var(--desc-color)]">
                  <div className="h-6 w-6 animate-spin rounded-full border-2 border-[var(--primary-color)] border-t-transparent" />
                  <span>加载中...</span>
                </div>
              ) : null}
              {scanStatus === 86038 || scanStatus === 86090 ? (
                <button
                  className="absolute inset-0 flex flex-col items-center justify-center gap-2 bg-white/90 text-sm text-[var(--desc-color)]"
                  onClick={initScan}
                >
                  <span>{scanStatus === 86038 ? "二维码已过期" : "扫码成功"}</span>
                  <span>{scanStatus === 86038 ? "点击刷新" : "请在手机上确认"}</span>
                </button>
              ) : null}
            </div>
            <div className="text-xs text-[var(--desc-color)]">请使用 Bilibili 客户端扫码</div>
            <div className="text-sm font-semibold text-[var(--content-color)]">
              {statusTextMap[String(scanStatus)] || "等待扫描..."}
            </div>
          </div>

          <div className="flex flex-1 flex-col gap-4">
            <div className="flex gap-8 text-base font-semibold text-[var(--desc-color)]">
              <button
                className={loginTab === "pwd" ? "text-[var(--primary-color)]" : ""}
                onClick={() => setLoginTab("pwd")}
              >
                账号登录
              </button>
              <button
                className={loginTab === "sms" ? "text-[var(--primary-color)]" : ""}
                onClick={() => setLoginTab("sms")}
              >
                短信登录
              </button>
            </div>

            {loginTab === "pwd" ? (
              <div className="flex flex-col gap-3 text-sm">
                <div className="flex items-center gap-3">
                  <span className="w-16 text-[var(--desc-color)]">账号</span>
                  <input
                    value={accountForm.username}
                    onChange={(event) =>
                      setAccountForm((prev) => ({ ...prev, username: event.target.value }))
                    }
                    placeholder="请输入账号"
                    className="flex-1"
                  />
                </div>
                <div className="flex items-center gap-3">
                  <span className="w-16 text-[var(--desc-color)]">密码</span>
                  <input
                    type="password"
                    value={accountForm.password}
                    onChange={(event) =>
                      setAccountForm((prev) => ({ ...prev, password: event.target.value }))
                    }
                    placeholder="请输入密码"
                    className="flex-1"
                  />
                </div>
                <button
                  className="h-10 rounded-lg bg-[var(--primary-color)] text-[var(--primary-text)]"
                  onClick={handlePwdLogin}
                  disabled={loginLoading}
                >
                  {loginLoading ? "登录中..." : "登录"}
                </button>
              </div>
            ) : (
              <div className="flex flex-col gap-3 text-sm">
                <div className="flex items-center gap-3">
                  <span className="w-16 text-[var(--desc-color)]">区号</span>
                  <select
                    value={smsForm.cid}
                    onChange={(event) =>
                      setSmsForm((prev) => ({ ...prev, cid: event.target.value }))
                    }
                    className="w-28"
                  >
                    <option value="">请选择</option>
                    {countries.map((item) => (
                      <option key={item.id} value={item.id}>
                        {item.name}
                      </option>
                    ))}
                  </select>
                  <input
                    value={smsForm.tel}
                    onChange={(event) =>
                      setSmsForm((prev) => ({ ...prev, tel: event.target.value }))
                    }
                    placeholder="请输入手机号"
                    className="flex-1"
                  />
                  <button
                    className="h-8 px-3 rounded-lg"
                    onClick={handleSendSms}
                    disabled={sendingSms}
                  >
                    {sendingSms ? "发送中" : "发送验证码"}
                  </button>
                </div>
                <div className="flex items-center gap-3">
                  <span className="w-16 text-[var(--desc-color)]">验证码</span>
                  <input
                    value={smsForm.code}
                    onChange={(event) =>
                      setSmsForm((prev) => ({ ...prev, code: event.target.value }))
                    }
                    placeholder="请输入短信验证码"
                    className="flex-1"
                  />
                </div>
                <button
                  className="h-10 rounded-lg bg-[var(--primary-color)] text-[var(--primary-text)]"
                  onClick={handleSmsLogin}
                  disabled={loginLoading}
                >
                  {loginLoading ? "登录中..." : "登录"}
                </button>
              </div>
            )}
            <div className="text-xs text-[var(--desc-color)]">
              登录即代表你同意数据使用规则
            </div>
          </div>
        </div>
      ) : (
        <div className="panel p-4">
          <div className="h-24 w-full rounded-lg bg-gradient-to-r from-sky-200/70 to-slate-200/70" />
          <div className="-mt-8 flex flex-wrap items-center gap-4">
            <div className="h-16 w-16 overflow-hidden rounded-full border-4 border-white bg-white">
              {avatarPreview ? (
                <img
                  src={avatarPreview}
                  alt="用户头像"
                  className="h-full w-full object-cover"
                  onError={() => {
                    invokeCommand("auth_client_log", {
                      message: `avatar_img_error:${String(avatarPreview).length}`,
                    }).catch(() => {});
                    setAvatarPreview("");
                  }}
                />
              ) : (
                <span className="avatar-fallback" />
              )}
            </div>
            <div>
              <div className="text-lg font-semibold text-[var(--content-color)]">{user.name}</div>
              <div className="text-xs text-[var(--desc-color)]">{user.desc}</div>
            </div>
          </div>
          <div className="mt-4 grid gap-3 text-sm text-[var(--content-color)] sm:grid-cols-4">
            <div className="rounded-lg bg-white/80 px-3 py-2 text-center">
              <div className="text-xs text-[var(--desc-color)]">关注数</div>
              <div className="font-semibold">{user.stat.following}</div>
            </div>
            <div className="rounded-lg bg-white/80 px-3 py-2 text-center">
              <div className="text-xs text-[var(--desc-color)]">粉丝数</div>
              <div className="font-semibold">{user.stat.follower}</div>
            </div>
            <div className="rounded-lg bg-white/80 px-3 py-2 text-center">
              <div className="text-xs text-[var(--desc-color)]">动态数</div>
              <div className="font-semibold">{user.stat.dynamic}</div>
            </div>
            <div className="rounded-lg bg-white/80 px-3 py-2 text-center">
              <div className="text-xs text-[var(--desc-color)]">硬币数</div>
              <div className="font-semibold">{user.stat.coins}</div>
            </div>
          </div>
        </div>
      )}

      {message ? (
        <div className="rounded-lg border border-amber-200 bg-amber-50 px-3 py-2 text-sm text-amber-700">
          {message}
        </div>
      ) : null}
    </div>
  );
}
