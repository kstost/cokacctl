/* Shared UI widgets */

const { useState, useEffect, useRef, useCallback, useMemo } = React;

// ─── Sparkline ─────────────────────────────────────────────
const Sparkline = ({ data, color = 'var(--accent)', height = 30 }) => {
  const w = 120, h = height;
  if (!data.length) return <svg width={w} height={h}/>;
  const min = Math.min(...data), max = Math.max(...data);
  const range = max - min || 1;
  const step = w / (data.length - 1 || 1);
  const points = data.map((v, i) => `${i * step},${h - ((v - min) / range) * (h - 4) - 2}`).join(' ');
  const area = `0,${h} ${points} ${w},${h}`;
  const gradId = 'sp-' + Math.random().toString(36).slice(2, 7);
  return (
    <svg className="spark" width="100%" height={h} viewBox={`0 0 ${w} ${h}`} preserveAspectRatio="none">
      <defs>
        <linearGradient id={gradId} x1="0" x2="0" y1="0" y2="1">
          <stop offset="0%" stopColor={color} stopOpacity="0.3"/>
          <stop offset="100%" stopColor={color} stopOpacity="0"/>
        </linearGradient>
      </defs>
      <polygon points={area} fill={`url(#${gradId})`}/>
      <polyline points={points} fill="none" stroke={color} strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round"/>
    </svg>
  );
};

// ─── Metric tile ───────────────────────────────────────────
const Metric = ({ label, value, unit, iconName, trend, trendValue, spark, sparkColor }) => (
  <div className="metric">
    <div className="k">
      {iconName && <Icon name={iconName} size={13}/>}
      <span>{label}</span>
    </div>
    <div className="v">
      {value}{unit && <span className="unit">{unit}</span>}
    </div>
    {spark && <Sparkline data={spark} color={sparkColor || 'var(--accent)'}/>}
    {trend && (
      <div className={`trend ${trend}`}>
        <Icon name={trend === 'up' ? 'arrowUp' : 'arrowDown'} size={11}/>
        {trendValue}
      </div>
    )}
  </div>
);

// ─── Spinner ──────────────────────────────────────────────
// Inherits currentColor so it matches whatever button/context it lives in
// (primary buttons: dark text color, danger buttons: red, etc.).
const Spinner = ({ size = 14 }) => (
  <span
    className="spinner"
    style={{ width: size, height: size }}
    role="status"
    aria-label="loading"
  />
);

// ─── Status tag ────────────────────────────────────────────
const StatusTag = ({ status }) => {
  const map = {
    running:      { tone: 'green',  label: 'Running' },
    stopped:      { tone: 'red',    label: 'Stopped' },
    starting:     { tone: 'amber',  label: 'Starting…' },
    stopping:     { tone: 'amber',  label: 'Stopping…' },
    restarting:   { tone: 'amber',  label: 'Restarting…' },
    removing:     { tone: 'amber',  label: 'Removing…' },
    'not-installed': { tone: '',    label: 'Not installed' },
  };
  const s = map[status] || { tone: '', label: status };
  return <span className={`tag ${s.tone}`}><span className="dot"/>{s.label}</span>;
};

// ─── Rendered log line ────────────────────────────────────
const renderLogMsg = (msg) => {
  const parts = [];
  const re = /<(n|s)>([^<]+)<\/\1>/g;
  let last = 0, m;
  while ((m = re.exec(msg)) !== null) {
    if (m.index > last) parts.push(msg.slice(last, m.index));
    parts.push(<span key={m.index} className={m[1]}>{m[2]}</span>);
    last = m.index + m[0].length;
  }
  if (last < msg.length) parts.push(msg.slice(last));
  return parts;
};

const LogLine = ({ line }) => (
  <div className="log-line">
    <span className="t">{fmtTime(line.time)}</span>
    <span className={`lvl ${line.level}`}>{line.level}</span>
    <span className="src">{line.source}</span>
    <span className="msg">{renderLogMsg(line.msg)}</span>
  </div>
);

// ─── Toast ────────────────────────────────────────────────
const ToastZone = ({ toasts }) => (
  <div className="toast-zone">
    {toasts.map(t => (
      <div key={t.id} className={`toast ${t.kind}`}>
        <Icon name={t.kind === 'err' ? 'warn' : 'check'} size={16}/>
        <span>{t.msg}</span>
      </div>
    ))}
  </div>
);

Object.assign(window, {
  Sparkline, Metric, Spinner, StatusTag, LogLine, ToastZone, renderLogMsg,
});
