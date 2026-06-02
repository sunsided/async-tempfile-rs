use std::hash::{BuildHasher, Hasher, RandomState};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

/// Strictly monotonic per-process counter. Guarantees that two names generated
/// within the same process never collide, even when created at the same instant.
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Represents a randomly generated file name.
pub(crate) struct RandomName {
    name: String,
}

impl RandomName {
    // Only used by the non-`uuid` code path; the `uuid` feature generates names
    // from UUIDv4 instead, leaving this dead outside of tests.
    #[cfg_attr(feature = "uuid", allow(dead_code))]
    pub fn new(prefix: &str) -> Self {
        let pid = std::process::id();

        // OS-seeded entropy: `RandomState`'s keys are derived from the operating
        // system RNG, so hashing empty input yields ~64 random bits per call.
        // This makes generated names unpredictable (no symlink/preexisting-file
        // races) without pulling in an RNG dependency.
        let entropy = RandomState::new().build_hasher().finish();

        // Strictly monotonic within the process for guaranteed local uniqueness.
        let counter = COUNTER.fetch_add(1, Ordering::Relaxed);

        // Wall-clock time for additional cross-process variation.
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or(std::time::Duration::from_secs(0));
        let (secs, subsec_nanos) = (now.as_secs(), now.subsec_nanos());

        Self {
            name: format!("{prefix}{pid:x}{secs:x}{subsec_nanos:x}{entropy:x}{counter:x}"),
        }
    }

    pub fn as_str(&self) -> &str {
        self.name.as_str()
    }
}

impl AsRef<str> for RandomName {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_random_name() {
        let first = RandomName::new("test");
        let second = RandomName::new("test");
        assert!(first.as_str().starts_with("test"));
        assert!(second.as_str().starts_with("test"));
        assert_ne!(first.as_str(), second.as_str());
    }

    /// Property guard for the name generator: across many rapid, same-prefix
    /// calls every name must be distinct (no collisions from the monotonic
    /// counter or entropy mixing) and carry the requested prefix. Guards against
    /// regressions in the counter / entropy / format wiring.
    #[test]
    fn names_are_unique_and_prefixed_over_many_iterations() {
        use std::collections::HashSet;

        const ITERATIONS: usize = 10_000;
        let mut seen = HashSet::with_capacity(ITERATIONS);
        for _ in 0..ITERATIONS {
            let name = RandomName::new("px_");
            assert!(
                name.as_str().starts_with("px_"),
                "missing prefix: {}",
                name.as_str()
            );
            assert!(
                seen.insert(name.as_str().to_string()),
                "duplicate name generated: {}",
                name.as_str()
            );
        }
    }

    /// Affixes containing a path separator must be flagged unsafe so they can be
    /// rejected before reaching the filesystem (see `crate::affix_is_safe`).
    #[test]
    fn separator_bearing_affixes_are_unsafe() {
        assert!(crate::affix_is_safe("ok_"));
        assert!(crate::affix_is_safe(".log"));
        assert!(crate::affix_is_safe("")); // empty is fine
        assert!(crate::affix_is_safe("..")); // no separator -> still a plain name fragment

        assert!(!crate::affix_is_safe("../"));
        assert!(!crate::affix_is_safe("a/b"));
        assert!(!crate::affix_is_safe("/etc/passwd"));
    }
}
