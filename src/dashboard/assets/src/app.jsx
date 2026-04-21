/* Main app — wired to cokacctl's local REST API on loopback. */

/* ── auth bootstrap ─────────────────────────────────────────
   Both loopback and inbound modes emit a URL like
     http(s)://<host>:<port>/#access=<hex-token>
   The fragment never reaches the server, so we capture it on load,
   stash it in sessionStorage, and immediately scrub it out of the
   URL bar / history. The token is then attached as
     Authorization: Bearer <token>
   on every API call. In loopback it's a defense-in-depth layer on
   top of the Host allowlist; in inbound it's the primary auth. */
const AUTH_KEY = 'cokacctl-auth';
function captureAuthFromHash() {
  try {
    const m = (location.hash || '').match(/access=([A-Za-z0-9]+)/);
    if (!m) return;
    sessionStorage.setItem(AUTH_KEY, m[1]);
    history.replaceState(null, '', location.pathname + location.search);
  } catch {}
}
captureAuthFromHash();
function getAuthToken() {
  try { return sessionStorage.getItem(AUTH_KEY) || ''; } catch { return ''; }
}
function setAuthToken(t) {
  try {
    if (t) sessionStorage.setItem(AUTH_KEY, t);
    else   sessionStorage.removeItem(AUTH_KEY);
  } catch {}
}

const NAV = [
  { group: null, items: [
    { id: 'overview', label: 'Overview', icon: 'home' },
  ]},
  { group: 'Operations', items: [
    { id: 'service',  label: 'Service',  icon: 'server' },
    { id: 'bots',     label: 'Bots',     icon: 'bot' },
    { id: 'tokens',   label: 'Tokens',   icon: 'key' },
  ]},
  { group: 'Monitoring', items: [
    { id: 'logs',     label: 'Logs',     icon: 'logs' },
    { id: 'activity', label: 'Activity', icon: 'activity' },
  ]},
  { group: 'System', items: [
    { id: 'updates',  label: 'Updates',  icon: 'update' },
    { id: 'settings', label: 'Settings', icon: 'settings' },
  ]},
];

const PAGE_LABELS = Object.fromEntries(NAV.flatMap(g => g.items.map(i => [i.id, i.label])));

const api = async (method, path, body) => {
  const headers = body ? { 'Content-Type': 'application/json' } : {};
  const tok = getAuthToken();
  if (tok) headers['Authorization'] = 'Bearer ' + tok;
  const resp = await fetch(path, {
    method,
    headers,
    body: body ? JSON.stringify(body) : undefined,
  });
  const text = await resp.text();
  let data = null;
  if (text) {
    try { data = JSON.parse(text); } catch { data = { error: text }; }
  }
  if (resp.status === 401) {
    setAuthToken('');
    const err = new Error((data && data.error) || 'Unauthorized');
    err.unauthorized = true;
    throw err;
  }
  if (!resp.ok) {
    const err = (data && data.error) || resp.statusText || `HTTP ${resp.status}`;
    throw new Error(err);
  }
  return data;
};

const parseState = (raw) => {
  if (!raw) return null;
  return {
    ...raw,
    startedAt: raw.startedAt ? new Date(raw.startedAt) : null,
    lastCheck: raw.lastCheck ? new Date(raw.lastCheck) : null,
    bots: (raw.bots || []).map(b => ({
      ...b,
      addedAt: b.addedAt ? new Date(b.addedAt) : null,
    })),
  };
};

const parseLogs = (arr) => (arr || []).map(l => ({
  ...l,
  time: l.time ? new Date(l.time) : new Date(),
}));

const parseActivity = (arr) => (arr || []).map(a => ({
  ...a,
  when: a.when ? new Date(a.when) : new Date(),
}));

const App = () => {
  const [page, setPage] = useState(() => {
    try { return localStorage.getItem('cokacctl-page') || 'overview'; } catch { return 'overview'; }
  });
  useEffect(() => {
    try { localStorage.setItem('cokacctl-page', page); } catch {}
  }, [page]);

  const [sidebarOpen, setSidebarOpen] = useState(false);
  const goTo = useCallback((id) => { setPage(id); setSidebarOpen(false); }, []);

  const [state, setState] = useState(null);
  const [logs, setLogs] = useState([]);
  const [activity, setActivity] = useState([]);
  // `pendingAction` carries the action key ('start', 'stop', 'restart',
  // 'remove', 'install', 'update') of whichever long-running request is in
  // flight, or null. Pages use it to pick which specific button shows the
  // spinner while also disabling the rest; the simple boolean `busy` derived
  // below preserves the old "any action running" check used as a disable
  // guard throughout the UI.
  const [pendingAction, setPendingAction] = useState(null);
  const busy = pendingAction !== null;
  const [toasts, setToasts] = useState([]);
  const [authNeeded, setAuthNeeded] = useState(false);
  const [pendingToken, setPendingToken] = useState('');

  const toast = useCallback((msg, kind = 'ok') => {
    const id = 't-' + Date.now() + '-' + Math.random().toString(36).slice(2,5);
    setToasts(t => [...t, { id, msg, kind }]);
    setTimeout(() => setToasts(t => t.filter(x => x.id !== id)), 2800);
  }, []);

  const refreshState = useCallback(async () => {
    try {
      const raw = await api('GET', '/api/state');
      setState(parseState(raw));
      setAuthNeeded(false);
    } catch (e) {
      if (e.unauthorized) setAuthNeeded(true);
      else console.error('state fetch failed', e);
    }
  }, []);

  const refreshLogs = useCallback(async () => {
    try {
      const raw = await api('GET', '/api/logs');
      setLogs(parseLogs(raw.lines));
    } catch (e) {
      if (e.unauthorized) setAuthNeeded(true);
      else console.error('logs fetch failed', e);
    }
  }, []);

  const refreshActivity = useCallback(async () => {
    try {
      const raw = await api('GET', '/api/activity');
      setActivity(parseActivity(raw.items));
    } catch (e) {
      if (e.unauthorized) setAuthNeeded(true);
      else console.error('activity fetch failed', e);
    }
  }, []);

  useEffect(() => {
    refreshState();
    refreshLogs();
    refreshActivity();
  }, []);

  useEffect(() => {
    const id = setInterval(() => { refreshState(); refreshActivity(); }, 2000);
    return () => clearInterval(id);
  }, [refreshState, refreshActivity]);

  useEffect(() => {
    if (page !== 'logs') return;
    const id = setInterval(refreshLogs, 1500);
    return () => clearInterval(id);
  }, [page, refreshLogs]);

  const runAction = useCallback(async (actionKey, label, method, path, body) => {
    setPendingAction(actionKey);
    try {
      const data = await api(method, path, body);
      toast(data?.message || `${label} done`, 'ok');
      refreshState();
      refreshActivity();
      refreshLogs();
      return data;
    } catch (e) {
      toast(`${label} failed: ${e.message}`, 'err');
      throw e;
    } finally {
      setPendingAction(null);
    }
  }, [toast, refreshState, refreshActivity, refreshLogs]);

  const actions = useMemo(() => ({
    goto: goTo,

    start:   () => runAction('start',   'Start service',    'POST', '/api/service/start').catch(()=>{}),
    stop:    () => runAction('stop',    'Stop service',     'POST', '/api/service/stop').catch(()=>{}),
    restart: () => runAction('restart', 'Restart service',  'POST', '/api/service/restart').catch(()=>{}),
    remove:  () => runAction('remove',  'Remove service',   'POST', '/api/service/remove').catch(()=>{}),

    install: () => runAction('install', 'Install cokacdir', 'POST', '/api/install').catch(()=>{}),
    update:  () => runAction('update',  'Update',           'POST', '/api/update/apply').catch(()=>{}),

    checkUpdate: async () => {
      try {
        const data = await api('POST', '/api/update/check');
        refreshState();
        if (data.latestVersion && state?.cokacdirVersion && data.latestVersion !== state.cokacdirVersion) {
          toast(`Update available: v${data.latestVersion}`, 'ok');
        } else if (data.latestVersion) {
          toast('Already on the latest version', 'ok');
        } else {
          toast('Could not fetch version info', 'err');
        }
      } catch (e) {
        toast(`Update check failed: ${e.message}`, 'err');
      }
    },

    addBot: async (token, name) => {
      try {
        await api('POST', '/api/tokens/add', { token, name });
        toast('Bot added. Restart the service for it to take effect.', 'ok');
        refreshState();
        refreshActivity();
      } catch (e) {
        toast(`Add failed: ${e.message}`, 'err');
      }
    },
    toggleBot: async (id) => {
      try {
        const data = await api('POST', '/api/tokens/toggle', { id });
        toast(data.disabled ? 'Bot disabled' : 'Bot enabled', 'ok');
        refreshState();
        refreshActivity();
      } catch (e) {
        toast(`Change failed: ${e.message}`, 'err');
      }
    },
    removeBot: async (id) => {
      try {
        await api('POST', '/api/tokens/delete', { id });
        toast('Bot removed', 'ok');
        refreshState();
        refreshActivity();
      } catch (e) {
        toast(`Remove failed: ${e.message}`, 'err');
      }
    },

    setBinaryPath: async (path) => {
      try {
        await api('POST', '/api/binary-path', { path });
        toast('Binary path saved', 'ok');
        refreshState();
      } catch (e) {
        toast(`Save failed: ${e.message}`, 'err');
      }
    },
  }), [runAction, refreshState, refreshActivity, toast, state, goTo]);

  if (authNeeded) {
    const submit = (e) => {
      e?.preventDefault?.();
      const t = pendingToken.trim();
      if (!t) return;
      setAuthToken(t);
      setAuthNeeded(false);
      setPendingToken('');
      refreshState();
      refreshLogs();
      refreshActivity();
    };
    return (
      <div style={{ height: '100vh', display: 'grid', placeItems: 'center', background: 'var(--bg)', padding: 20 }}>
        <form onSubmit={submit} style={{
          maxWidth: 460, width: '100%', background: 'var(--bg-card)',
          border: '1px solid var(--line)', borderRadius: 'var(--r-lg)',
          padding: 28, boxShadow: '0 8px 32px rgba(0,0,0,.4)',
        }}>
          <h2 style={{ margin: '0 0 6px' }}>Access token required</h2>
          <div style={{ color: 'var(--fg-dim)', marginBottom: 18, fontSize: 13 }}>
            Paste the <code>#access=…</code> fragment from the URL printed in the cokacctl terminal, or enter just the token.
          </div>
          <input
            autoFocus
            className="input"
            placeholder="Access token"
            value={pendingToken}
            onChange={(e) => setPendingToken(e.target.value.replace(/.*access=/, ''))}
            style={{ width: '100%', padding: '10px 12px', background: 'var(--bg-2)', border: '1px solid var(--line-2)', borderRadius: 'var(--r)', color: 'var(--fg)', fontFamily: 'var(--mono)' }}
          />
          <button type="submit" className="btn primary" style={{ marginTop: 14, width: '100%' }} disabled={!pendingToken.trim()}>
            Connect
          </button>
        </form>
      </div>
    );
  }

  if (!state) {
    return (
      <div style={{ height: '100vh', display: 'grid', placeItems: 'center', color: 'var(--fg-dim)' }}>
        Loading…
      </div>
    );
  }

  const fullState = { ...state, logs, activity, busy, pendingAction };
  const pageEl = (() => {
    switch (page) {
      case 'overview': return <OverviewPage state={fullState} actions={actions}/>;
      case 'service':  return <ServicePage  state={fullState} actions={actions}/>;
      case 'bots':     return <BotsPage     state={fullState} actions={actions}/>;
      case 'tokens':   return <TokensPage   state={fullState} actions={actions} toast={toast}/>;
      case 'logs':     return <LogsPage     state={fullState}/>;
      case 'updates':  return <UpdatesPage  state={fullState} actions={actions}/>;
      case 'activity': return <ActivityPage state={fullState}/>;
      case 'settings': return <SettingsPage state={fullState} actions={actions} toast={toast}/>;
      default:         return <OverviewPage state={fullState} actions={actions}/>;
    }
  })();

  const { serviceStatus, cokacdirVersion, latestVersion, bots, platform, cokacctlVersion } = state;
  const activeBots = bots.filter(b => !b.disabled).length;

  return (
    <div className="shell" data-screen-label={`cokacctl · ${PAGE_LABELS[page] || page}`}>
      <aside className={`sidebar ${sidebarOpen ? 'open' : ''}`}>
        <div className="brand">
          <div className="mark">C</div>
          <div className="name">cokacctl</div>
          <div className="ver">v{cokacctlVersion}</div>
        </div>

        {NAV.map((group, gi) => (
          <div className="nav-group" key={gi}>
            {group.group && <div className="nav-label">{group.group}</div>}
            {group.items.map(item => {
              let badge = null;
              if (item.id === 'bots' && bots.length > 0) badge = <span className="badge">{activeBots}</span>;
              if (item.id === 'updates' && cokacdirVersion && latestVersion && cokacdirVersion !== latestVersion) {
                badge = <span className="badge amber">1</span>;
              }
              if (item.id === 'service') {
                if (serviceStatus === 'running') badge = <span className="badge" style={{color:'var(--green)',borderColor:'var(--green-border)',background:'var(--green-soft)'}}>●</span>;
                else if (serviceStatus === 'stopped') badge = <span className="badge red">●</span>;
              }
              return (
                <div key={item.id}
                     className="nav-item"
                     aria-current={page === item.id ? 'page' : undefined}
                     onClick={() => goTo(item.id)}>
                  <Icon name={item.icon} size={15}/>
                  <span>{item.label}</span>
                  {badge}
                </div>
              );
            })}
          </div>
        ))}

        <div className="sidebar-foot">
          <div className="host">
            <span className="dot" style={{
              background: serviceStatus === 'running' ? 'var(--green)' : 'var(--fg-dim)',
              boxShadow: serviceStatus === 'running' ? '0 0 8px var(--green)' : 'none',
            }}/>
            <span>{platform.host}</span>
          </div>
          <div>{platform.os} · {platform.label}</div>
        </div>
      </aside>

      <div className={`sidebar-backdrop ${sidebarOpen ? 'open' : ''}`}
           onClick={() => setSidebarOpen(false)}/>

      <main className="main">
        <div className="topbar">
          <button className="mobile-menu-btn"
                  aria-label="Open menu"
                  onClick={() => setSidebarOpen(true)}>
            <Icon name="menu" size={18}/>
          </button>
          <div className="crumb">
            <span>cokacctl</span>
            <span className="sep">/</span>
            <span className="current">{PAGE_LABELS[page] || page}</span>
          </div>
          <div className="topbar-actions">
            <StatusTag status={serviceStatus}/>
            <button className="btn ghost sm" onClick={() => goTo('logs')}>
              <Icon name="terminal" size={14}/> Logs
            </button>
          </div>
        </div>
        <div className="content">
          <div className="content-inner">
            {pageEl}
          </div>
        </div>
      </main>

      <ToastZone toasts={toasts}/>
    </div>
  );
};

ReactDOM.createRoot(document.getElementById('root')).render(<App/>);
