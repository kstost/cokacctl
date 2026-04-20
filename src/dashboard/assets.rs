//! Embedded static assets for the dashboard.
//!
//! Vendor JS (React / ReactDOM / Babel standalone) is bundled into the binary
//! so the dashboard works fully offline. Without this, the HTML would pull
//! these scripts from unpkg.com on first render and an air-gapped / offline
//! host would see a blank page.

pub const DASHBOARD_HTML: &str = include_str!("assets/Dashboard.html");
pub const ICONS_JSX:   &str = include_str!("assets/src/icons.jsx");
pub const DATA_JSX:    &str = include_str!("assets/src/data.jsx");
pub const WIDGETS_JSX: &str = include_str!("assets/src/widgets.jsx");
pub const PAGES_JSX:   &str = include_str!("assets/src/pages.jsx");
pub const APP_JSX:     &str = include_str!("assets/src/app.jsx");

pub const REACT_JS:     &str = include_str!("assets/vendor/react.production.min.js");
pub const REACT_DOM_JS: &str = include_str!("assets/vendor/react-dom.production.min.js");
pub const BABEL_JS:     &str = include_str!("assets/vendor/babel.min.js");

/// Returns `(content_type, body)` for a given request path, or `None`.
pub fn lookup(path: &str) -> Option<(&'static str, &'static str)> {
    let p = path.split('?').next().unwrap_or(path);
    match p {
        "/" | "/index.html" | "/Dashboard.html" =>
            Some(("text/html; charset=utf-8", DASHBOARD_HTML)),
        "/src/icons.jsx"   => Some(("application/javascript; charset=utf-8", ICONS_JSX)),
        "/src/data.jsx"    => Some(("application/javascript; charset=utf-8", DATA_JSX)),
        "/src/widgets.jsx" => Some(("application/javascript; charset=utf-8", WIDGETS_JSX)),
        "/src/pages.jsx"   => Some(("application/javascript; charset=utf-8", PAGES_JSX)),
        "/src/app.jsx"     => Some(("application/javascript; charset=utf-8", APP_JSX)),
        "/vendor/react.js"     => Some(("application/javascript; charset=utf-8", REACT_JS)),
        "/vendor/react-dom.js" => Some(("application/javascript; charset=utf-8", REACT_DOM_JS)),
        "/vendor/babel.js"     => Some(("application/javascript; charset=utf-8", BABEL_JS)),
        _ => None,
    }
}
