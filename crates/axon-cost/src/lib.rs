//! `axon-cost` — cost ledger, provider profiles, reports.
//!
//! Stage 16 surface for §56:
//!
//!   * [`ProviderProfile`] — per-model price rates (`input_per_million`,
//!     `output_per_million`, `per_call_cents`).
//!   * [`Ledger`] — append-only list of [`CostEntry`] records, one per
//!     API call. Persists to JSON.
//!   * [`Report`] — summary built from a `Ledger` + `ProviderProfile`s:
//!     total cents, per-provider breakdown, p50/p95 latency, top-N most
//!     expensive calls, hourly buckets.
//!
//! Cents-not-dollars everywhere so we don't have to deal with float
//! pennies in money fields.

pub mod cache;
mod entry;
mod ledger;
mod profile;
mod report;

pub use cache::{CacheStats, CachedEntry, PrefixCache, PrefixCacheKey};
pub use entry::CostEntry;
pub use ledger::{Ledger, LedgerError};
pub use profile::ProviderProfile;
pub use report::{ProviderSummary, Report, TopCall};
