export function formatNumber(value) {
  if (value === null || value === undefined) {
    return "-";
  }
  return new Intl.NumberFormat("en-US").format(value);
}

export function formatDuration(seconds) {
  if (!Number.isFinite(seconds)) {
    return "00:00";
  }
  const totalSeconds = Math.max(0, Math.floor(seconds));
  const hrs = Math.floor(totalSeconds / 3600);
  const mins = Math.floor((totalSeconds % 3600) / 60);
  const secs = totalSeconds % 60;
  if (hrs > 0) {
    return `${String(hrs).padStart(2, "0")}:${String(mins).padStart(2, "0")}:${String(secs).padStart(2, "0")}`;
  }
  return `${String(mins).padStart(2, "0")}:${String(secs).padStart(2, "0")}`;
}

export function formatDateTime(value) {
  if (!value) {
    return "-";
  }
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }
  return date.toLocaleString();
}

export function parseVideoInput(input) {
  if (!input) {
    return { bvid: null, aid: null };
  }
  const bvidMatch = input.match(/BV[0-9A-Za-z]+/);
  if (bvidMatch) {
    return { bvid: bvidMatch[0], aid: null };
  }
  const aidMatch = input.match(/av(\d+)/i);
  if (aidMatch) {
    return { bvid: null, aid: Number(aidMatch[1]) };
  }
  if (/^\d+$/.test(input.trim())) {
    return { bvid: null, aid: Number(input.trim()) };
  }
  return { bvid: null, aid: null };
}
