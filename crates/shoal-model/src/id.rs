//! Ids for sessions, tabs, panes and splits.
//!
//! Each id is a `u64` inside and a prefixed string on the wire: `"s3"`, `"t12"`,
//! `"p40"`, `"x7"`. All four kinds share one counter, [`Workspace::next_id`].
//!
//! [`Workspace::next_id`]: crate::Workspace::next_id

use std::fmt;
use std::str::FromStr;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A string that is not a valid id of the expected kind.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid {kind} id {input:?}: expected \"{prefix}\" followed by a number")]
pub struct ParseIdError {
    kind: &'static str,
    prefix: char,
    input: String,
}

/// Parses `<prefix><digits>`, rejecting signs and leading zeros so every id has
/// exactly one spelling.
fn parse_raw(input: &str, prefix: char) -> Option<u64> {
    let digits = input.strip_prefix(prefix)?;
    let canonical = match digits.as_bytes() {
        [] => false,
        [b'0', _, ..] => false,
        bytes => bytes.iter().all(u8::is_ascii_digit),
    };
    if canonical { digits.parse().ok() } else { None }
}

macro_rules! define_id {
    ($(#[$doc:meta])* $name:ident, $prefix:literal, $kind:literal) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u64);

        impl $name {
            pub const PREFIX: char = $prefix;

            pub const fn new(raw: u64) -> Self {
                Self(raw)
            }

            /// The numeric id, as binary frames carry it.
            pub const fn get(self) -> u64 {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}{}", Self::PREFIX, self.0)
            }
        }

        impl FromStr for $name {
            type Err = ParseIdError;

            fn from_str(input: &str) -> Result<Self, Self::Err> {
                parse_raw(input, Self::PREFIX).map(Self).ok_or_else(|| ParseIdError {
                    kind: $kind,
                    prefix: Self::PREFIX,
                    input: input.to_owned(),
                })
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                struct IdVisitor;

                impl Visitor<'_> for IdVisitor {
                    type Value = $name;

                    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                        write!(f, "a {} id such as \"{}1\"", $kind, $prefix)
                    }

                    fn visit_str<E: de::Error>(self, value: &str) -> Result<$name, E> {
                        value.parse().map_err(E::custom)
                    }
                }

                deserializer.deserialize_str(IdVisitor)
            }
        }
    };
}

define_id!(
    /// A session, `"s3"` on the wire.
    SessionId, 's', "session"
);
define_id!(
    /// A tab, `"t12"` on the wire.
    TabId, 't', "tab"
);
define_id!(
    /// A pane, `"p40"` on the wire.
    PaneId, 'p', "pane"
);
define_id!(
    /// A split node in a tab's layout, `"x7"` on the wire.
    SplitId, 'x', "split"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_canonical_ids() {
        assert_eq!("s3".parse(), Ok(SessionId::new(3)));
        assert_eq!("t12".parse(), Ok(TabId::new(12)));
        assert_eq!("p0".parse(), Ok(PaneId::new(0)));
        assert_eq!("x18446744073709551615".parse(), Ok(SplitId::new(u64::MAX)));
    }

    #[test]
    fn rejects_malformed_ids() {
        for input in [
            "",
            "p",
            "40",
            "s40",
            "P40",
            "p+4",
            "p-4",
            "p04",
            "p 4",
            "p4 ",
            "p4x",
            "p٤",
            "p18446744073709551616",
        ] {
            assert!(input.parse::<PaneId>().is_err(), "accepted {input:?}");
        }
    }

    #[test]
    fn displays_with_prefix() {
        assert_eq!(PaneId::new(40).to_string(), "p40");
        assert_eq!(SplitId::new(7).to_string(), "x7");
    }

    #[test]
    fn parse_error_names_kind_and_input() {
        let err = "t9".parse::<SessionId>().unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid session id \"t9\": expected \"s\" followed by a number"
        );
    }
}
