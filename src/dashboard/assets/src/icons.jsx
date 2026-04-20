/* Tiny icon set — stroke-based, 16px default. */
const Icon = ({ name, size = 16, className = '', style }) => {
  const P = {
    strokeLinecap: 'round', strokeLinejoin: 'round',
    strokeWidth: 1.75, stroke: 'currentColor', fill: 'none',
  };
  const paths = {
    home:    <><path {...P} d="M3 10.5 12 3l9 7.5"/><path {...P} d="M5 9.5V21h14V9.5"/></>,
    server:  <><rect {...P} x="3.5" y="4" width="17" height="7" rx="1.5"/><rect {...P} x="3.5" y="13" width="17" height="7" rx="1.5"/><circle cx="7" cy="7.5" r="0.9" fill="currentColor"/><circle cx="7" cy="16.5" r="0.9" fill="currentColor"/></>,
    bot:     <><rect {...P} x="4" y="7" width="16" height="12" rx="2.5"/><path {...P} d="M12 3v4"/><circle {...P} cx="9" cy="13" r="1.2"/><circle {...P} cx="15" cy="13" r="1.2"/><path {...P} d="M2 13h2M20 13h2"/></>,
    key:     <><circle {...P} cx="8" cy="15" r="4"/><path {...P} d="m11 12 9-9M16 7l3 3M14 9l3 3"/></>,
    logs:    <><path {...P} d="M4 5h16M4 10h16M4 15h10M4 20h12"/></>,
    update:  <><path {...P} d="M3 12a9 9 0 0 1 15.5-6.3L21 8"/><path {...P} d="M21 3v5h-5"/><path {...P} d="M21 12a9 9 0 0 1-15.5 6.3L3 16"/><path {...P} d="M3 21v-5h5"/></>,
    activity:<><path {...P} d="M3 12h4l3-7 4 14 3-7h4"/></>,
    settings:<><circle {...P} cx="12" cy="12" r="3"/><path {...P} d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.9 2.9l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 0 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.6 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.9-2.9l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 0 1 0-4h.1A1.7 1.7 0 0 0 4.7 9a1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.9-2.9l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 0 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.9 2.9l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 0 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"/></>,
    play:    <><path {...P} d="M7 5v14l12-7z"/></>,
    stop:    <><rect {...P} x="5.5" y="5.5" width="13" height="13" rx="1.5"/></>,
    restart: <><path {...P} d="M3 12a9 9 0 1 0 3-6.7"/><path {...P} d="M3 3v6h6"/></>,
    trash:   <><path {...P} d="M4 7h16M10 4h4a1 1 0 0 1 1 1v2H9V5a1 1 0 0 1 1-1zM6 7l1 13a2 2 0 0 0 2 2h6a2 2 0 0 0 2-2l1-13"/><path {...P} d="M10 11v7M14 11v7"/></>,
    plus:    <><path {...P} d="M12 5v14M5 12h14"/></>,
    check:   <><path {...P} d="m5 12 5 5 9-11"/></>,
    x:       <><path {...P} d="M6 6l12 12M18 6L6 18"/></>,
    arrowUp: <><path {...P} d="M12 19V5M6 11l6-6 6 6"/></>,
    arrowDown:<><path {...P} d="M12 5v14M6 13l6 6 6-6"/></>,
    info:    <><circle {...P} cx="12" cy="12" r="9"/><path {...P} d="M12 8h.01M11 12h1v5h1"/></>,
    warn:    <><path {...P} d="M10.3 3.9 1.8 18.4A2 2 0 0 0 3.5 21.4h17a2 2 0 0 0 1.7-3l-8.5-14.5a2 2 0 0 0-3.4 0z"/><path {...P} d="M12 9v5M12 18h.01"/></>,
    pause:   <><rect {...P} x="6" y="5" width="4" height="14" rx="1"/><rect {...P} x="14" y="5" width="4" height="14" rx="1"/></>,
    copy:    <><rect {...P} x="8" y="8" width="12" height="12" rx="2"/><path {...P} d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2"/></>,
    eye:     <><path {...P} d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12z"/><circle {...P} cx="12" cy="12" r="3"/></>,
    eyeOff:  <><path {...P} d="M17.9 17.9A10.5 10.5 0 0 1 12 19c-6.5 0-10-7-10-7a19 19 0 0 1 4.2-5M9.9 4.2A10.5 10.5 0 0 1 12 4c6.5 0 10 7 10 7a19 19 0 0 1-3.1 4.2M1 1l22 22"/><path {...P} d="M14.1 14.1A3 3 0 0 1 9.9 9.9"/></>,
    folder:  <><path {...P} d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/></>,
    download:<><path {...P} d="M12 3v13M6 10l6 6 6-6M4 21h16"/></>,
    terminal:<><path {...P} d="M5 8l4 4-4 4M13 16h6"/><rect {...P} x="2" y="4" width="20" height="16" rx="2"/></>,
    clock:   <><circle {...P} cx="12" cy="12" r="9"/><path {...P} d="M12 7v5l3 2"/></>,
    link:    <><path {...P} d="M10 14a5 5 0 0 0 7 0l3-3a5 5 0 0 0-7-7l-1 1"/><path {...P} d="M14 10a5 5 0 0 0-7 0l-3 3a5 5 0 0 0 7 7l1-1"/></>,
    sparkle: <><path {...P} d="M12 3v4M12 17v4M3 12h4M17 12h4M6 6l2.5 2.5M15.5 15.5 18 18M6 18l2.5-2.5M15.5 8.5 18 6"/></>,
    cpu:     <><rect {...P} x="4" y="4" width="16" height="16" rx="2"/><rect {...P} x="9" y="9" width="6" height="6"/><path {...P} d="M9 1v3M15 1v3M9 20v3M15 20v3M20 9h3M20 14h3M1 9h3M1 14h3"/></>,
    menu:    <><path {...P} d="M4 6h16M4 12h16M4 18h16"/></>,
  };
  const el = paths[name];
  if (!el) return null;
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" className={`ico ${className}`} style={style} aria-hidden="true">{el}</svg>
  );
};

window.Icon = Icon;
