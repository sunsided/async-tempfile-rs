use std::time::SystemTime;

/// Represents a randomly generated file name.
pub(crate) struct RandomName {
    name: String,
}

impl RandomName {
    #[allow(dead_code)]
    pub fn new(prefix: &str) -> Self {
        let pid = std::process::id();

        // Using the address of a local variable for extra variation.
        let marker = &pid as *const _ as usize;

        // Current timestamp for added variation.
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or(std::time::Duration::from_secs(0));
        let (secs, subsec_nanos) = (now.as_secs(), now.subsec_nanos());

        Self {
            name: format!("{}{}{:x}{:x}{:x}", prefix, pid, marker, secs, subsec_nanos),
        }
    }
}

impl AsRef<str> for RandomName {
    fn as_ref(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_random_name() {
        let name = RandomName::new("test");
        assert!(name.as_ref().starts_with("test"))
    }
}
