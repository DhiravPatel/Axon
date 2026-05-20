//! `axon-deploy` — production deploy primitives.
//!
//! Stage 17 surface for §41:
//!
//!   * [`http`] — a minimal HTTP/1.1 server. Pure-Rust, dep-free, thread-
//!     per-connection. Routes `POST /invoke`, `GET /healthz`,
//!     `GET /readyz`. Not a hyper replacement; designed to be readable in
//!     one sitting and good enough to expose an Axon program at the edge
//!     of a service mesh that handles the heavy lifting (load balancing,
//!     TLS, retries).
//!   * [`HealthCheck`] — `name() + check() -> CheckResult` trait, with
//!     two built-ins: [`AlwaysHealthy`] and [`Liveness`] (just returns
//!     ok). Tests show how to plug a custom check in.
//!   * [`dotenv`] — `.env` file loader that respects existing process env
//!     by default (no clobber) so secrets baked at deploy time win over
//!     repo defaults.
//!   * [`DeployManifest`] — the `deploy.json` sibling of `.axskill`:
//!     `port`, `entrypoint_handler`, `env: BTreeMap<String, String>`,
//!     `health_checks: Vec<String>`. Pairs with `axon-skill` so a deploy
//!     is `manifest + skill` in one folder.

pub mod dotenv;
pub mod health;
pub mod http;
pub mod manifest;
pub mod metrics;
pub mod serverless;

pub use health::{AlwaysHealthy, CheckResult, HealthCheck, Liveness};
pub use http::{Request, Response, Server};
pub use manifest::DeployManifest;
pub use metrics::MetricsRegistry;
pub use serverless::{ServerlessTarget, ServerlessTrampoline};
