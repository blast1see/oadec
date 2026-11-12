//! How much a finding weighs.

/// How much a finding weighs.
///
/// A run fails when the worst finding is at or above the threshold it was
/// given (`verify --fail-on`); the default threshold is [`Severity::Error`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// Worth knowing; says nothing against the stream.
    Info,
    /// Outside what the rule's source asks for, or a rule not confirmed
    /// independently; the audio is not known to be affected.
    Warning,
    /// The stream breaks a rule it is known to have to keep; what was
    /// delivered from it is not trustworthy.
    Error,
    /// The stream could not be read past this point as it stands.
    Fatal,
}

impl Severity {
    /// Every severity, lightest first.
    pub const ALL: [Self; 4] = [Self::Info, Self::Warning, Self::Error, Self::Fatal];

    /// The name used on the command line and in reports.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
            Self::Fatal => "fatal",
        }
    }

    /// The severity of a name, as [`Severity::name`] writes it.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.name() == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severities_are_ordered_and_named_both_ways() {
        assert!(Severity::Info < Severity::Warning);
        assert!(Severity::Warning < Severity::Error);
        assert!(Severity::Error < Severity::Fatal);
        for s in Severity::ALL {
            assert_eq!(Severity::from_name(s.name()), Some(s));
        }
        assert_eq!(Severity::from_name("bad"), None);
    }
}
