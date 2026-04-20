/* Page components — Overview, Service, Bots, Tokens, Logs, Updates, Activity, Settings */

// ─── Overview ─────────────────────────────────────────────
const OverviewPage = ({ state, actions }) => {
  const { serviceStatus, cokacdirVersion, latestVersion, bots, startedAt, activity } = state;

  const uptimeMs = startedAt ? Date.now() - startedAt.getTime() : 0;
  const pulseCls = serviceStatus === 'running' ? '' : serviceStatus === 'stopped' || serviceStatus === 'not-installed' ? 'stopped' : 'unknown';
  const stateCls = serviceStatus === 'running' ? 'green' : serviceStatus === 'stopped' ? 'red' : 'amber';
  const stateLabel = {
    running: 'Running',
    stopped: 'Stopped',
    starting: 'Starting…',
    stopping: 'Stopping…',
    restarting: 'Restarting…',
    removing: 'Removing…',
    'not-installed': 'Not installed',
  }[serviceStatus] || serviceStatus;

  const activeBots = bots.filter(b => !b.disabled).length;
  const updateAvailable = cokacdirVersion && latestVersion && cokacdirVersion !== latestVersion;

  return (
    <>
      <div className="page-header">
        <div>
          <h1>Overview</h1>
          <div className="subtitle">cokacdir service status and key info at a glance.</div>
        </div>
        <div className="page-header-actions">
          <button className="btn" onClick={() => actions.checkUpdate()} disabled={state.checkingUpdate}>
            <Icon name="update" size={14}/> {state.checkingUpdate ? 'Checking…' : 'Check for updates'}
          </button>
        </div>
      </div>

      <div className="hero">
        <div className="hero-row">
          <div className="hero-status">
            <div className={`pulse ${pulseCls}`}><div className="core"/></div>
            <div>
              <div className="label">Service status</div>
              <div className={`state ${stateCls}`}>{stateLabel}</div>
            </div>
          </div>
          <div className="hero-stats">
            <div className="hero-stat">
              <div className="k">Uptime</div>
              <div className="v">{serviceStatus === 'running' && startedAt ? fmtUptime(uptimeMs) : '—'}</div>
            </div>
            <div className="hero-stat">
              <div className="k">Active bots</div>
              <div className="v">{activeBots}<span className="unit">/ {bots.length}</span></div>
            </div>
            <div className="hero-stat">
              <div className="k">Version</div>
              <div className="v">{cokacdirVersion ? `v${cokacdirVersion}` : '—'}</div>
            </div>
          </div>
        </div>
        <div className="hero-actions">
          {serviceStatus !== 'running' ? (
            <button className="btn primary" disabled={activeBots === 0 || state.busy} onClick={() => actions.start()}>
              <Icon name="play" size={14}/> Start service
            </button>
          ) : (
            <>
              <button className="btn" disabled={state.busy} onClick={() => actions.restart()}>
                <Icon name="restart" size={14}/> Restart
              </button>
              <button className="btn danger" disabled={state.busy} onClick={() => actions.stop()}>
                <Icon name="stop" size={14}/> Stop
              </button>
            </>
          )}
          {updateAvailable && (
            <button className="btn" disabled={state.busy} onClick={() => actions.update()}>
              <Icon name="download" size={14}/> Update to v{latestVersion}
            </button>
          )}
        </div>
      </div>

      <div className="grid g-2" style={{ marginTop: 20 }}>
        <div className="card">
          <div className="card-h">
            <h3>Registered bots</h3>
            <button className="btn ghost sm" onClick={() => actions.goto('bots')}>View all</button>
          </div>
          {bots.length === 0 ? (
            <div className="empty">
              No bots registered yet.
              <div style={{ marginTop: 10 }}>
                <button className="btn primary sm" onClick={() => actions.goto('tokens')}>
                  <Icon name="plus" size={14}/> Add your first bot
                </button>
              </div>
            </div>
          ) : (
            <div style={{ display: 'flex', flexDirection: 'column' }}>
              {bots.slice(0, 5).map((b, i) => (
                <div key={b.id} className="svc-row" style={{ padding: '12px 0', borderBottom: i === Math.min(bots.length, 5) - 1 ? 0 : '' }}>
                  <div className="bot-avatar" style={{ background: AVATAR_GRADIENTS[i % AVATAR_GRADIENTS.length], width: 36, height: 36, borderRadius: 10 }}>
                    {b.name.slice(0, 1)}
                  </div>
                  <div className="body">
                    <div className="title">{b.name}</div>
                    <div className="desc" style={{ fontFamily: 'var(--mono)' }}>{b.preview}</div>
                  </div>
                  {b.disabled ? <StatusTag status="stopped"/> : <StatusTag status="running"/>}
                </div>
              ))}
            </div>
          )}
        </div>
        <div className="card">
          <div className="card-h">
            <h3>Recent events</h3>
            <button className="btn ghost sm" onClick={() => actions.goto('activity')}>View all</button>
          </div>
          {activity.length === 0 ? (
            <div className="empty">No events recorded yet.</div>
          ) : (
            <div className="activity-feed">
              {activity.slice(0, 5).map(a => (
                <ActivityRow key={a.id} item={a} compact/>
              ))}
            </div>
          )}
        </div>
      </div>
    </>
  );
};

// ─── Service ─────────────────────────────────────────────
const ServicePage = ({ state, actions }) => {
  const { serviceStatus, platform, bots, startedAt } = state;
  const busy = state.busy || ['starting','stopping','restarting','removing'].includes(serviceStatus);
  const running = serviceStatus === 'running';
  const activeBots = bots.filter(b => !b.disabled).length;

  return (
    <>
      <div className="page-header">
        <div>
          <h1>Service</h1>
          <div className="subtitle">
            Manage cokacdir as a background service via {platform.label}.
          </div>
        </div>
        <StatusTag status={serviceStatus}/>
      </div>

      <div className="card" style={{ padding: 0 }}>
        <div className="svc-row">
          <div className={`ico-wrap ${running ? 'green' : ''}`}>
            <Icon name="play" size={16}/>
          </div>
          <div className="body">
            <div className="title">Start service</div>
            <div className="desc">Run cokacdir with the {activeBots} active bot token(s) and register it to start automatically at boot.</div>
          </div>
          <button className="btn primary" disabled={busy || running || activeBots === 0} onClick={() => actions.start()}>
            <Icon name="play" size={14}/> Start
          </button>
        </div>

        <div className="svc-row">
          <div className={`ico-wrap ${busy ? 'amber' : ''}`}>
            <Icon name="restart" size={16}/>
          </div>
          <div className="body">
            <div className="title">Restart</div>
            <div className="desc">Restart the service with the currently registered tokens. Run this after adding or removing tokens so changes take effect.</div>
          </div>
          <button className="btn" disabled={busy || !running} onClick={() => actions.restart()}>
            <Icon name="restart" size={14}/> Restart
          </button>
        </div>

        <div className="svc-row">
          <div className={`ico-wrap ${!running && serviceStatus === 'stopped' ? 'red' : ''}`}>
            <Icon name="stop" size={16}/>
          </div>
          <div className="body">
            <div className="title">Stop</div>
            <div className="desc">Gracefully stop the service process. It won't auto-start on the next reboot.</div>
          </div>
          <button className="btn" disabled={busy || !running} onClick={() => actions.stop()}>
            <Icon name="stop" size={14}/> Stop
          </button>
        </div>

        <div className="svc-row">
          <div className="ico-wrap">
            <Icon name="trash" size={16}/>
          </div>
          <div className="body">
            <div className="title">Remove service</div>
            <div className="desc">Fully unregister cokacdir from {platform.label}. It will be re-registered on the next start.</div>
          </div>
          <button className="btn danger" disabled={busy || serviceStatus === 'not-installed'} onClick={() => actions.remove()}>
            <Icon name="trash" size={14}/> Remove
          </button>
        </div>
      </div>

      <div className="grid g-2" style={{ marginTop: 20 }}>
        <div className="card">
          <div className="card-h"><h3>Runtime info</h3></div>
          <div className="kv-row">
            <div className="k">Platform backend<div className="k-sub">Service manager</div></div>
            <div className="v">{platform.label}</div>
          </div>
          <div className="kv-row">
            <div className="k">Host</div>
            <div className="v">{platform.host} · {platform.os}</div>
          </div>
          <div className="kv-row">
            <div className="k">Binary</div>
            <div className="v">{state.binaryPath || '—'}</div>
          </div>
          <div className="kv-row">
            <div className="k">Started at</div>
            <div className="v">{running && startedAt ? fmtDate(startedAt) : '—'}</div>
          </div>
          <div className="kv-row">
            <div className="k">Active tokens</div>
            <div className="v">{activeBots} / {bots.length}</div>
          </div>
        </div>
        <div className="card">
          <div className="card-h"><h3>Quick tips</h3></div>
          <div style={{ fontSize: 13, color: 'var(--fg-mid)', lineHeight: 1.65 }}>
            <p style={{ marginTop: 0 }}>• After adding, removing, or disabling a token, <b>restart</b> for changes to take effect.</p>
            <p>• <b>Remove</b> only unregisters from {platform.label}; it doesn't delete the binary or config.</p>
            <p>• Service logs go to <span className="code">{state.logPath}</span>.</p>
            <p style={{ marginBottom: 0 }}>• The service can't start without at least one active bot token.</p>
          </div>
        </div>
      </div>
    </>
  );
};

// ─── Bots ─────────────────────────────────────────────────
const BotsPage = ({ state, actions }) => {
  const { bots } = state;
  return (
    <>
      <div className="page-header">
        <div>
          <h1>Bots</h1>
          <div className="subtitle">Status of registered bots. Each bot is tied to one Telegram token.</div>
        </div>
        <div className="page-header-actions">
          <button className="btn primary" onClick={() => actions.goto('tokens')}>
            <Icon name="plus" size={14}/> Add bot
          </button>
        </div>
      </div>

      <div className="grid g-2">
        {bots.map((b, i) => (
          <div key={b.id} className={`bot-card ${b.disabled ? 'disabled' : ''}`}>
            <div className="bot-head">
              <div className="bot-avatar" style={{ background: AVATAR_GRADIENTS[i % AVATAR_GRADIENTS.length] }}>
                {b.name.slice(0, 1)}
              </div>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div className="bot-name">{b.name}</div>
                <div className="bot-handle">{b.handle}</div>
              </div>
              {b.disabled ? <StatusTag status="stopped"/> : <StatusTag status="running"/>}
            </div>

            <div className="bot-foot" style={{ borderTop: 0, marginTop: 0, paddingTop: 0 }}>
              <span className="token">{b.preview}</span>
              <div style={{ display: 'flex', gap: 6 }}>
                <button className="btn ghost sm" onClick={() => actions.toggleBot(b.id)}>
                  {b.disabled ? 'Enable' : 'Disable'}
                </button>
                <button className="btn ghost sm" onClick={() => actions.goto('tokens')}>Manage</button>
              </div>
            </div>
          </div>
        ))}
      </div>

      {bots.length === 0 && (
        <div className="card">
          <div className="empty">
            <Icon name="bot" size={24} style={{ marginBottom: 8, opacity: 0.6 }}/>
            <div>No bots registered yet.</div>
            <button className="btn primary" style={{ marginTop: 12 }} onClick={() => actions.goto('tokens')}>
              <Icon name="plus" size={14}/> Add your first bot
            </button>
          </div>
        </div>
      )}
    </>
  );
};

// ─── Tokens ──────────────────────────────────────────────
const TokensPage = ({ state, actions, toast }) => {
  const { bots } = state;
  const [input, setInput] = useState('');
  const [name, setName] = useState('');

  const submit = () => {
    const t = input.trim();
    if (!t) return;
    if (!/^\d+:[A-Za-z0-9_-]{20,}$/.test(t)) {
      toast('Invalid token format', 'err');
      return;
    }
    actions.addBot(t, name.trim());
    setInput(''); setName('');
  };

  return (
    <>
      <div className="page-header">
        <div>
          <h1>Tokens</h1>
          <div className="subtitle">
            Register and manage Telegram bot tokens. Get a token from{' '}
            <span className="code">@BotFather</span> via <span className="code">/newbot</span>.
          </div>
        </div>
      </div>

      <div className="card" style={{ marginBottom: 20 }}>
        <div className="card-h">
          <h3>Add new token</h3>
          <span className="sub">Press Enter to submit</span>
        </div>
        <div className="token-form">
          <div className="field" style={{ marginBottom: 0 }}>
            <div className="lbl">Display name (optional)</div>
            <input className="input" style={{ fontFamily: 'var(--sans)' }}
                   placeholder="e.g. Walk reminder bot"
                   value={name} onChange={(e) => setName(e.target.value)}/>
          </div>
          <div className="field" style={{ marginBottom: 0 }}>
            <div className="lbl">Bot token</div>
            <input className="input"
                   placeholder="123456789:AAH_9v-KcwJxPZrqT_B7m4LzNfYdX3wQp8Y"
                   value={input} onChange={(e) => setInput(e.target.value)}
                   onKeyDown={(e) => e.key === 'Enter' && submit()}/>
          </div>
          <button className="btn primary" onClick={submit} disabled={!input.trim()}>
            <Icon name="plus" size={14}/> Add
          </button>
        </div>
      </div>

      <div className="card" style={{ padding: 0, overflow: 'hidden' }}>
       <div className="table-scroll">
        <table className="table tokens-table">
          <thead>
            <tr>
              <th style={{ width: 50 }}></th>
              <th>Name / handle</th>
              <th>Token</th>
              <th>Status</th>
              <th style={{ textAlign: 'right', width: 160 }}></th>
            </tr>
          </thead>
          <tbody>
            {bots.map((b, i) => (
              <tr key={b.id}>
                <td>
                  <div className="bot-avatar" style={{ background: AVATAR_GRADIENTS[i % AVATAR_GRADIENTS.length], width: 28, height: 28, borderRadius: 7, fontSize: 12 }}>
                    {b.name.slice(0, 1)}
                  </div>
                </td>
                <td>
                  <div style={{ fontWeight: 500 }}>{b.name}</div>
                  <div style={{ color: 'var(--fg-dim)', fontSize: 12, fontFamily: 'var(--mono)' }}>{b.handle}</div>
                </td>
                <td className="mono">
                  <div style={{ display: 'flex', alignItems: 'center', gap: 4, whiteSpace: 'nowrap' }}>
                    <span>{b.preview}</span>
                  </div>
                </td>
                <td>{b.disabled ? <StatusTag status="stopped"/> : <StatusTag status="running"/>}</td>
                <td style={{ textAlign: 'right', whiteSpace: 'nowrap' }}>
                  <div style={{ display: 'inline-flex', gap: 6, alignItems: 'center' }}>
                    <button className="btn ghost sm" onClick={() => actions.toggleBot(b.id)}>
                      {b.disabled ? 'Enable' : 'Disable'}
                    </button>
                    <button className="btn ghost sm" onClick={() => actions.removeBot(b.id)}>
                      <Icon name="trash" size={12}/>
                    </button>
                  </div>
                </td>
              </tr>
            ))}
            {bots.length === 0 && (
              <tr><td colSpan="5"><div className="empty">No tokens registered yet.</div></td></tr>
            )}
          </tbody>
        </table>
       </div>
      </div>

      <div style={{ display: 'flex', gap: 8, marginTop: 14, alignItems: 'center', color: 'var(--fg-dim)', fontSize: 12.5 }}>
        <Icon name="info" size={14}/>
        <span>Token changes take effect after you <b style={{ color: 'var(--fg-mid)' }}>restart</b> the service. Disabled tokens stay registered but are excluded from the service.</span>
      </div>
    </>
  );
};

// ─── Logs ────────────────────────────────────────────────
const LogsPage = ({ state }) => {
  const { logs, logPath, serviceStatus } = state;
  const ref = useRef(null);
  const [follow, setFollow] = useState(true);

  useEffect(() => {
    if (follow && ref.current) ref.current.scrollTop = ref.current.scrollHeight;
  }, [logs, follow]);

  return (
    <>
      <div className="page-header">
        <div>
          <h1>Logs</h1>
          <div className="subtitle">Live log output from the cokacdir process.</div>
        </div>
        <div className="page-header-actions">
          <button className={`btn ${follow ? 'primary' : ''}`} onClick={() => setFollow(f => !f)}>
            <Icon name={follow ? 'pause' : 'play'} size={14}/> {follow ? 'Auto-scroll' : 'Paused'}
          </button>
        </div>
      </div>

      <div style={{ display: 'flex', alignItems: 'center', gap: 10, color: 'var(--fg-dim)', fontSize: 12.5, marginBottom: 12 }}>
        <Icon name="folder" size={14}/>
        <span className="code">{logPath || '—'}</span>
        <span style={{ marginLeft: 'auto' }}>
          <StatusTag status={serviceStatus}/>
        </span>
      </div>

      <div className="logs" ref={ref} style={{ height: '62vh', minHeight: 400 }}>
        {logs.length === 0 ? (
          <div className="empty">No logs yet.</div>
        ) : logs.map(l => <LogLine key={l.id} line={l}/>)}
      </div>
    </>
  );
};

// ─── Updates ─────────────────────────────────────────────
const UpdatesPage = ({ state, actions }) => {
  const { cokacctlVersion, cokacdirVersion, latestVersion, checkingUpdate, lastCheck } = state;
  const installed = cokacdirVersion;
  const updateAvailable = installed && latestVersion && installed !== latestVersion;
  const notInstalled = !installed;

  return (
    <>
      <div className="page-header">
        <div>
          <h1>Updates</h1>
          <div className="subtitle">Manage the cokacdir binary version.</div>
        </div>
        <button className="btn" onClick={() => actions.checkUpdate()} disabled={checkingUpdate}>
          <Icon name="update" size={14}/> {checkingUpdate ? 'Checking…' : 'Check now'}
        </button>
      </div>

      <div className="card" style={{ marginBottom: 20 }}>
        <div className="update-compare">
          <div className="col">
            <div style={{ fontSize: 12, color: 'var(--fg-dim)', textTransform: 'uppercase', letterSpacing: '0.08em' }}>
              Installed version
            </div>
            <div style={{ fontSize: 26, fontWeight: 600, fontFamily: 'var(--mono)', marginTop: 4 }}>
              {notInstalled ? 'Not installed' : `v${installed}`}
            </div>
            <div style={{ color: 'var(--fg-dim)', fontSize: 12, marginTop: 4 }}>
              Last check: {lastCheck ? fmtRelative(lastCheck) : '—'}
            </div>
          </div>
          <div className="arrow">→</div>
          <div className="col">
            <div style={{ fontSize: 12, color: 'var(--fg-dim)', textTransform: 'uppercase', letterSpacing: '0.08em' }}>
              Latest version
            </div>
            <div style={{
              fontSize: 26, fontWeight: 600, fontFamily: 'var(--mono)', marginTop: 4,
              color: updateAvailable ? 'var(--amber)' : 'var(--green)',
            }}>
              {latestVersion ? `v${latestVersion}` : '—'}
            </div>
            <div style={{ fontSize: 12, marginTop: 4 }}>
              {notInstalled ? (
                <span style={{ color: 'var(--fg-dim)' }}>Not installed</span>
              ) : updateAvailable ? (
                <span style={{ color: 'var(--amber)' }}>Update available</span>
              ) : latestVersion ? (
                <span style={{ color: 'var(--green)' }}>Up to date</span>
              ) : (
                <span style={{ color: 'var(--fg-dim)' }}>Check required</span>
              )}
            </div>
          </div>
          <div className="action">
            {notInstalled ? (
              <button className="btn primary" onClick={() => actions.install()} disabled={state.busy}>
                <Icon name="download" size={14}/> Install cokacdir
              </button>
            ) : updateAvailable ? (
              <button className="btn primary" onClick={() => actions.update()} disabled={state.busy}>
                <Icon name="download" size={14}/> Update to v{latestVersion}
              </button>
            ) : (
              <button className="btn" disabled>
                <Icon name="check" size={14}/> Up to date
              </button>
            )}
          </div>
        </div>
        {updateAvailable && (
          <div style={{
            marginTop: 16, padding: '10px 14px',
            background: 'var(--amber-soft)', border: '1px solid var(--amber-border)',
            borderRadius: 'var(--r-sm)', fontSize: 12.5, color: 'var(--fg-mid)',
            display: 'flex', alignItems: 'center', gap: 10,
          }}>
            <Icon name="info" size={14} style={{ color: 'var(--amber)' }}/>
            <span>The service is automatically stopped during the update and restarted afterward.</span>
          </div>
        )}
      </div>

      <div className="card">
        <div className="card-h"><h3>Installation info</h3></div>
        <div className="kv-row">
          <div className="k">cokacctl<div className="k-sub">Management tool serving this dashboard</div></div>
          <div className="v">v{cokacctlVersion}</div>
        </div>
        <div className="kv-row">
          <div className="k">cokacdir<div className="k-sub">Managed daemon</div></div>
          <div className="v">{installed ? `v${installed}` : 'Not installed'}</div>
        </div>
        <div className="kv-row">
          <div className="k">Binary path</div>
          <div className="v">{state.binaryPath || '—'}</div>
        </div>
      </div>
    </>
  );
};

// ─── Activity ────────────────────────────────────────────
const ACTIVITY_ICONS = {
  'svc-start':   'play',
  'svc-stop':    'stop',
  'svc-restart': 'restart',
  'bot-add':     'bot',
  'bot-disable': 'eyeOff',
  'bot-remove':  'trash',
  'update':      'download',
  'install':     'download',
  'warn':        'warn',
};

const ActivityRow = ({ item, compact }) => (
  <div className="activity-item" style={compact ? { padding: '10px 0' } : undefined}>
    <div className={`ico-wrap ${item.tone || ''}`}>
      <Icon name={ACTIVITY_ICONS[item.kind] || 'info'} size={14}/>
    </div>
    <div className="body">
      <div className="title">{item.title}</div>
      <div className="meta">{item.meta}</div>
    </div>
    <div className="t">{fmtRelative(item.when)}</div>
  </div>
);

const ActivityPage = ({ state }) => {
  const { activity } = state;
  const [filter, setFilter] = useState('all');
  const filtered = useMemo(() => {
    if (filter === 'all') return activity;
    if (filter === 'service') return activity.filter(a => a.kind.startsWith('svc-'));
    if (filter === 'bots') return activity.filter(a => a.kind.startsWith('bot-'));
    if (filter === 'system') return activity.filter(a => ['update','install','warn'].includes(a.kind));
    return activity;
  }, [activity, filter]);

  const filters = [
    ['all', 'All', activity.length],
    ['service', 'Service', activity.filter(a => a.kind.startsWith('svc-')).length],
    ['bots', 'Bots', activity.filter(a => a.kind.startsWith('bot-')).length],
    ['system', 'System', activity.filter(a => ['update','install','warn'].includes(a.kind)).length],
  ];

  return (
    <>
      <div className="page-header">
        <div>
          <h1>Activity</h1>
          <div className="subtitle">Events recorded in cokacctl during this session.</div>
        </div>
      </div>

      <div style={{ display: 'flex', gap: 6, marginBottom: 16 }}>
        {filters.map(([id, label, count]) => (
          <button key={id} className={`btn ${filter === id ? '' : 'ghost'} sm`}
                  onClick={() => setFilter(id)}
                  style={filter === id ? { background: 'var(--bg-2)', borderColor: 'var(--line-hover)' } : {}}>
            {label} <span style={{ color: 'var(--fg-faint)', marginLeft: 4 }}>{count}</span>
          </button>
        ))}
      </div>

      <div className="card">
        <div className="activity-feed">
          {filtered.map(a => <ActivityRow key={a.id} item={a}/>)}
          {filtered.length === 0 && <div className="empty">No matching events.</div>}
        </div>
      </div>
    </>
  );
};

// ─── Settings ───────────────────────────────────────────
const SettingsPage = ({ state, actions, toast }) => {
  const [binPath, setBinPath] = useState(state.binaryPath || '');

  useEffect(() => { setBinPath(state.binaryPath || ''); }, [state.binaryPath]);

  return (
    <>
      <div className="page-header">
        <div>
          <h1>Settings</h1>
          <div className="subtitle">Configure cokacctl / cokacdir paths and behavior.</div>
        </div>
      </div>

      <div className="card" style={{ marginBottom: 20 }}>
        <div className="card-h">
          <h3>cokacdir binary</h3>
        </div>

        <div className="field">
          <div className="lbl">Binary path</div>
          <div className="hint">
            Absolute path to the cokacdir executable. Leave empty to auto-detect from PATH and default locations.
          </div>
          <div style={{ display: 'flex', gap: 8 }}>
            <input className="input" value={binPath} onChange={(e) => setBinPath(e.target.value)}
                   placeholder="/usr/local/bin/cokacdir" style={{ minWidth: 0 }}/>
            <button className="btn" style={{ flexShrink: 0, whiteSpace: 'nowrap' }} onClick={() => {
              actions.setBinaryPath(binPath);
            }}>Save</button>
          </div>
        </div>

        <div className="divider"/>

        <div className="field" style={{ marginBottom: 0 }}>
          <div className="lbl">Service manager</div>
          <div className="hint">Determined automatically from the host OS.</div>
          <div style={{ marginTop: 6, fontFamily: 'var(--mono)', fontSize: 13 }}>
            {state.platform.os} · <span style={{ color: 'var(--accent-2)' }}>{state.platform.label}</span>
          </div>
        </div>
      </div>

      <div className="card">
        <div className="card-h">
          <h3>File locations</h3>
          <span className="sub">Read-only</span>
        </div>
        <div className="kv-row">
          <div className="k">Config file<div className="k-sub">Tokens, binary path</div></div>
          <div className="v">{state.configPath}</div>
        </div>
        <div className="kv-row">
          <div className="k">Service log</div>
          <div className="v">{state.logPath || '—'}</div>
        </div>
        <div className="kv-row">
          <div className="k">Error log</div>
          <div className="v">{state.errorLogPath || '—'}</div>
        </div>
        <div className="kv-row">
          <div className="k">Debug log</div>
          <div className="v">{state.debugLogPath}</div>
        </div>
      </div>
    </>
  );
};

Object.assign(window, {
  OverviewPage, ServicePage, BotsPage, TokensPage, LogsPage, UpdatesPage, ActivityPage, SettingsPage,
  ActivityRow,
});
