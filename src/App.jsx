import { useEffect, useMemo, useState } from "react";
import AnchorSection from "./sections/AnchorSection";
import DownloadSection from "./sections/DownloadSection";
import SubmissionSection from "./sections/SubmissionSection";
import SettingsSection from "./sections/SettingsSection";
import BiliToolsLoginSection from "./sections/BiliToolsLoginSection";
import ToolboxSection from "./sections/ToolboxSection";
import AboutSection from "./sections/AboutSection";
import { invokeCommand } from "./lib/tauri";

const sections = [
  { id: "anchor", label: "主播订阅", short: "订" },
  { id: "download", label: "视频下载", short: "下" },
  { id: "submission", label: "视频投稿", short: "投" },
  {
    id: "toolbox",
    label: "工具箱",
    short: "工",
    children: [{ id: "toolbox.remux", label: "格式转码" }],
  },
  { id: "settings", label: "设置", short: "设" },
  { id: "about", label: "关于", short: "关" },
];

const sectionLabels = {
  auth: "登录",
  anchor: "主播订阅",
  download: "视频下载",
  submission: "视频投稿",
  toolbox: "工具箱",
  "toolbox.remux": "格式转码",
  settings: "设置",
  about: "关于",
};

function App() {
  const [active, setActive] = useState("download");
  const [expandedMenus, setExpandedMenus] = useState({ toolbox: false });
  const [authStatus, setAuthStatus] = useState({ loggedIn: false });
  const [avatarPreview, setAvatarPreview] = useState("");

  const activeSection = useMemo(() => active.split(".")[0], [active]);

  const activeLabel = useMemo(() => {
    if (active.includes(".")) {
      const parent = active.split(".")[0];
      const parentLabel = sectionLabels[parent] || "";
      const childLabel = sectionLabels[active] || "";
      return parentLabel && childLabel ? `${parentLabel} / ${childLabel}` : parentLabel || childLabel;
    }
    return sectionLabels[active] || "";
  }, [active]);

  const avatarUrl = useMemo(() => {
    const raw = authStatus?.userInfo || {};
    const level1 = raw?.data || raw;
    const level2 = level1?.data || level1;
    return (
      level2?.avatar ||
      level2?.face ||
      level1?.avatar ||
      level1?.face ||
      ""
    );
  }, [authStatus]);

  const refreshAuthStatus = async () => {
    try {
      const data = await invokeCommand("auth_status");
      setAuthStatus(data || { loggedIn: false });
    } catch (error) {
      setAuthStatus((prev) => prev || { loggedIn: false });
    }
  };

  useEffect(() => {
    refreshAuthStatus();
  }, []);

  useEffect(() => {
    const parent = active.split(".")[0];
    const hasChildren = sections.some((item) => item.id === parent && item.children?.length);
    if (!hasChildren) {
      return;
    }
    setExpandedMenus((prev) => {
      if (prev[parent]) {
        return prev;
      }
      return { ...prev, [parent]: true };
    });
  }, [active]);

  useEffect(() => {
    const loadAvatar = async () => {
      if (!authStatus?.loggedIn || !avatarUrl) {
        setAvatarPreview("");
        return;
      }
      try {
        const data = await invokeCommand("video_proxy_image", { url: avatarUrl });
        setAvatarPreview(data || "");
        await invokeCommand("auth_client_log", {
          message: `app_avatar_proxy_ok:${String(avatarUrl).length}:${String(data || "").length}`,
        });
      } catch (error) {
        const message = error?.message || String(error || "");
        await invokeCommand("auth_client_log", {
          message: `app_avatar_proxy_fail:${String(avatarUrl).length}:${message}`,
        });
      }
    };
    loadAvatar();
  }, [authStatus?.loggedIn, avatarUrl]);

  const renderSection = () => {
    switch (activeSection) {
      case "auth":
        return <BiliToolsLoginSection onStatusChange={setAuthStatus} initialStatus={authStatus} />;
      case "anchor":
        return <AnchorSection />;
      case "download":
        return <DownloadSection />;
      case "submission":
        return <SubmissionSection />;
      case "toolbox":
        return <ToolboxSection />;
      case "settings":
        return <SettingsSection />;
      case "about":
        return <AboutSection />;
      default:
        return null;
    }
  };

  return (
    <div className="app-shell">
      <aside className="sidebar">
        {sections.map((item) => {
          const hasChildren = Boolean(item.children?.length);
          const isParentActive = activeSection === item.id;
          if (!hasChildren) {
            return (
              <button
                key={item.id}
                className={activeSection === item.id ? "active" : ""}
                onClick={() => setActive(item.id)}
                title={item.label}
              >
                <span className="menu-label">{item.label}</span>
              </button>
            );
          }
          const expanded = Boolean(expandedMenus[item.id]);
          return (
            <div
              key={item.id}
              className={expanded ? "menu-group expanded" : "menu-group"}
            >
              <button
                className={isParentActive ? "active" : ""}
                onClick={() =>
                  setExpandedMenus((prev) => ({
                    ...prev,
                    [item.id]: !prev[item.id],
                  }))
                }
                title={item.label}
              >
                <span className="menu-label">{item.label}</span>
                <span className="menu-caret" />
              </button>
              {expanded ? (
                <div className="submenu">
                  {item.children.map((child) => (
                    <button
                      key={child.id}
                      className={active === child.id ? "active submenu-item" : "submenu-item"}
                      onClick={() => setActive(child.id)}
                      title={child.label}
                    >
                      <span className="menu-label">{child.label}</span>
                    </button>
                  ))}
                </div>
              ) : null}
            </div>
          );
        })}
      </aside>
      <div id="main" className="main-shell">
        <div className="title-bar" data-tauri-drag-region>
          <div className="flex items-center gap-3 text-[var(--content-color)]">
            <span className="text-sm font-semibold">Reaction Cut</span>
            <span className="text-xs text-[var(--desc-color)]">{activeLabel}</span>
          </div>
          <button
            className={`avatar-btn ${active === "auth" ? "active" : ""}`}
            onClick={() => setActive("auth")}
            title="登录"
            data-tauri-drag-region="false"
          >
            {authStatus?.loggedIn && avatarPreview ? (
              <img
                src={avatarPreview}
                alt="用户头像"
                onError={() => setAvatarPreview("")}
              />
            ) : (
              <span className="avatar-fallback" />
            )}
          </button>
        </div>
        <div className="content-wrap">
          <div className="page">
            <div className="page-scroll">{renderSection()}</div>
          </div>
        </div>
      </div>
    </div>
  );
}

export default App;
