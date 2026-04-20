/* Shared helpers */

const fmtDate = (d = new Date()) => {
  if (!d) return '—';
  const pad = (n) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${pad(d.getMonth()+1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
};
const fmtTime = (d = new Date()) => {
  if (!d) return '';
  const pad = (n) => String(n).padStart(2, '0');
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
};
const fmtRelative = (d) => {
  if (!d) return '—';
  const diff = Math.floor((Date.now() - d.getTime()) / 1000);
  if (diff < 10) return 'just now';
  if (diff < 60) return `${diff}s ago`;
  if (diff < 3600) return `${Math.floor(diff/60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff/3600)}h ago`;
  return `${Math.floor(diff/86400)}d ago`;
};
const fmtUptime = (ms) => {
  if (ms == null || ms < 0) return '—';
  const s = Math.floor(ms / 1000);
  const d = Math.floor(s / 86400);
  const h = Math.floor((s % 86400) / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  if (d > 0) return `${d}d ${h}h`;
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m ${sec}s`;
  return `${sec}s`;
};
const fmtNum = (n) => n == null ? '—' : Number(n).toLocaleString('en-US');

const maskToken = (t) => {
  if (!t) return '';
  if (t.length <= 16) return t;
  return t.slice(0, 8) + '·'.repeat(10) + t.slice(-6);
};

const AVATAR_GRADIENTS = [
  'linear-gradient(135deg, #f5b84a, #f16a6a)',
  'linear-gradient(135deg, #6e8cff, #b49cf0)',
  'linear-gradient(135deg, #34d399, #7ad8e6)',
  'linear-gradient(135deg, #f16a6a, #b49cf0)',
  'linear-gradient(135deg, #7ad8e6, #6e8cff)',
  'linear-gradient(135deg, #b49cf0, #f5b84a)',
];

Object.assign(window, {
  fmtDate, fmtTime, fmtRelative, fmtUptime, fmtNum, maskToken,
  AVATAR_GRADIENTS,
});
