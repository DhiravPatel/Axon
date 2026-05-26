//! Prompt-prefix cache (§56.1).
//!
//! Modern LLM providers (Anthropic, OpenAI, Gemini) offer **prompt-prefix
//! caching**: a stable, hashed prefix is stored server-side, and
//! subsequent calls that share the prefix pay a reduced rate for those
//! tokens and skip the encode latency. The provider is in charge of the
//! actual cache; our job is to:
//!
//!   1. *Mark* which prompt regions are stable so the provider sees a
//!      matching prefix (`PrefixCacheKey::new(text, ttl_secs)`).
//!   2. Track local hit/miss statistics so the cost ledger and the
//!      `axon prof --cost` report can show cache hit-rate, tokens saved,
//!      and per-call attribution.
//!
//! The cache itself is an in-memory `HashMap` keyed by a 64-bit FNV hash
//! of the canonical text. Entries expire after `ttl_secs` so stale
//! prompts naturally drop out. The hash is *not* cryptographic — it only
//! has to match locally, since the provider hashes the prefix itself
//! before keying its own store.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PrefixCacheKey(pub u64);

impl PrefixCacheKey {
    /// FNV-1a 64-bit hash. The text *must* be canonical — different
    /// whitespace produces different keys, which is correct: provider
    /// caches see the bytes you send.
    pub fn from_text(text: &str) -> Self {
        let mut hash: u64 = 0xcbf29ce484222325;
        for b in text.as_bytes() {
            hash ^= *b as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        PrefixCacheKey(hash)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedEntry {
    pub key: PrefixCacheKey,
    /// Token count of the cached region — used to compute the discounted
    /// portion of a subsequent request.
    pub tokens: u32,
    /// Inserted-at, nanoseconds since epoch.
    pub inserted_at_ns: i64,
    /// Expiry, nanoseconds since epoch. 0 = no expiry.
    pub expires_at_ns: i64,
    /// Number of times the key has been observed on lookups so far.
    pub hits: u64,
}

impl CachedEntry {
    pub fn is_expired(&self, now_ns: i64) -> bool {
        self.expires_at_ns != 0 && now_ns >= self.expires_at_ns
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheStats {
    pub lookups: u64,
    pub hits: u64,
    pub misses: u64,
    pub tokens_saved: u64,
    pub entries: u64,
}

impl CacheStats {
    pub fn hit_rate(&self) -> f64 {
        if self.lookups == 0 {
            0.0
        } else {
            self.hits as f64 / self.lookups as f64
        }
    }
}

/// Thread-safe in-memory prefix cache. Wrapped in a `Mutex` so the host
/// crate can install a single global instance reachable from any
/// `cache(...)` call site.
pub struct PrefixCache {
    inner: Mutex<CacheInner>,
}

#[derive(Default)]
struct CacheInner {
    entries: HashMap<PrefixCacheKey, CachedEntry>,
    stats: CacheStats,
}

impl Default for PrefixCache {
    fn default() -> Self {
        Self::new()
    }
}

impl PrefixCache {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(CacheInner::default()),
        }
    }

    /// Mark `text` as a stable prefix. Returns the key. If the entry
    /// already exists its TTL is refreshed.
    pub fn insert(
        &self,
        text: &str,
        tokens: u32,
        now_ns: i64,
        ttl_secs: i64,
    ) -> PrefixCacheKey {
        let key = PrefixCacheKey::from_text(text);
        let mut guard = self.inner.lock().expect("PrefixCache poisoned");
        let expires_at_ns = if ttl_secs > 0 {
            now_ns.saturating_add(ttl_secs.saturating_mul(1_000_000_000))
        } else {
            0
        };
        let entry = guard.entries.entry(key.clone()).or_insert(CachedEntry {
            key: key.clone(),
            tokens,
            inserted_at_ns: now_ns,
            expires_at_ns,
            hits: 0,
        });
        // Refresh TTL & tokens on re-insert.
        entry.tokens = tokens;
        entry.expires_at_ns = expires_at_ns;
        guard.stats.entries = guard.entries.len() as u64;
        key
    }

    /// Look up a prefix. If present and unexpired, increment hits and
    /// return `Some((entry.tokens, total_hits_so_far))`. Otherwise None.
    pub fn lookup(&self, text: &str, now_ns: i64) -> Option<(u32, u64)> {
        let key = PrefixCacheKey::from_text(text);
        let mut guard = self.inner.lock().expect("PrefixCache poisoned");
        guard.stats.lookups += 1;
        // Lift the values we need before mutating stats to satisfy borrow ck.
        let action = match guard.entries.get(&key) {
            Some(e) if !e.is_expired(now_ns) => Some((e.tokens, e.hits + 1)),
            Some(_) => None, // expired — handled below as a miss + sweep
            None => None,
        };
        if let Some((tokens, new_hits)) = action {
            let e = guard.entries.get_mut(&key).unwrap();
            e.hits = new_hits;
            guard.stats.hits += 1;
            guard.stats.tokens_saved =
                guard.stats.tokens_saved.saturating_add(tokens as u64);
            Some((tokens, new_hits))
        } else {
            // Sweep this single expired entry if it existed.
            if let Some(e) = guard.entries.get(&key).cloned() {
                if e.is_expired(now_ns) {
                    guard.entries.remove(&key);
                    guard.stats.entries = guard.entries.len() as u64;
                }
            }
            guard.stats.misses += 1;
            None
        }
    }

    pub fn stats(&self) -> CacheStats {
        self.inner.lock().expect("PrefixCache poisoned").stats.clone()
    }

    /// Drop all entries whose TTL has elapsed before `now_ns`.
    pub fn sweep(&self, now_ns: i64) -> u64 {
        let mut guard = self.inner.lock().expect("PrefixCache poisoned");
        let before = guard.entries.len();
        guard.entries.retain(|_, e| !e.is_expired(now_ns));
        let after = guard.entries.len();
        guard.stats.entries = after as u64;
        (before - after) as u64
    }

    pub fn clear(&self) {
        let mut guard = self.inner.lock().expect("PrefixCache poisoned");
        guard.entries.clear();
        guard.stats = CacheStats::default();
    }

    pub fn len(&self) -> usize {
        self.inner.lock().expect("PrefixCache poisoned").entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(secs: i64) -> i64 {
        secs * 1_000_000_000
    }

    #[test]
    fn insert_and_lookup_within_ttl_hits() {
        let c = PrefixCache::new();
        c.insert("system: be precise", 1000, ts(100), 60);
        let r = c.lookup("system: be precise", ts(120)).unwrap();
        assert_eq!(r.0, 1000);
        assert_eq!(r.1, 1);
        let s = c.stats();
        assert_eq!(s.hits, 1);
        assert_eq!(s.misses, 0);
        assert_eq!(s.tokens_saved, 1000);
    }

    #[test]
    fn lookup_after_ttl_misses_and_evicts() {
        let c = PrefixCache::new();
        c.insert("prefix", 500, ts(0), 10);
        assert!(c.lookup("prefix", ts(20)).is_none());
        assert_eq!(c.stats().misses, 1);
        assert_eq!(c.len(), 0, "expired entries are evicted");
    }

    #[test]
    fn distinct_texts_have_distinct_keys() {
        let a = PrefixCacheKey::from_text("foo");
        let b = PrefixCacheKey::from_text("bar");
        assert_ne!(a, b);
    }

    #[test]
    fn whitespace_matters_for_keying() {
        let a = PrefixCacheKey::from_text("hello world");
        let b = PrefixCacheKey::from_text("hello  world");
        assert_ne!(a, b, "different bytes -> different keys");
    }

    #[test]
    fn re_insert_refreshes_ttl() {
        let c = PrefixCache::new();
        c.insert("k", 100, ts(0), 5);
        // Re-insert with a longer TTL.
        c.insert("k", 100, ts(0), 100);
        assert!(c.lookup("k", ts(50)).is_some());
    }

    #[test]
    fn sweep_drops_only_expired() {
        let c = PrefixCache::new();
        c.insert("a", 1, ts(0), 5);
        c.insert("b", 1, ts(0), 1000);
        let dropped = c.sweep(ts(10));
        assert_eq!(dropped, 1);
        assert!(c.lookup("a", ts(10)).is_none());
        assert!(c.lookup("b", ts(10)).is_some());
    }

    #[test]
    fn hit_rate_computation() {
        let c = PrefixCache::new();
        c.insert("x", 50, ts(0), 100);
        let _ = c.lookup("x", ts(1));
        let _ = c.lookup("x", ts(2));
        let _ = c.lookup("missing", ts(3));
        let s = c.stats();
        assert_eq!(s.lookups, 3);
        assert_eq!(s.hits, 2);
        assert_eq!(s.misses, 1);
        assert!((s.hit_rate() - 2.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn zero_ttl_means_no_expiry() {
        let c = PrefixCache::new();
        c.insert("k", 1, ts(0), 0);
        assert!(c.lookup("k", ts(1_000_000)).is_some());
    }
}
