//! Local web dashboard for cokacctl.
//!
//! Hand-rolled minimal HTTP/1.1 server on the loopback interface. Serves the
//! embedded React prototype (Dashboard.html + src/*.jsx) and a small JSON API
//! that wraps the existing CLI actions.

mod api;
mod assets;
mod server;
mod state;
mod tls;

pub use server::serve;
