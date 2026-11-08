use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::num::ParseIntError;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdParseError {
    MissingSeparator {
        expected: &'static str,
        input: String,
    },
    WrongPrefix {
        expected: &'static str,
        found: String,
    },
    InvalidRaw {
        expected: &'static str,
        raw: String,
        source: ParseIntError,
    },
}

impl fmt::Display for IdParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingSeparator { expected, input } => {
                write!(f, "身份 {expected} 缺少 '-' 分隔符: {input}")
            }
            Self::WrongPrefix { expected, found } => {
                write!(f, "身份前缀应为 {expected}，收到 {found}")
            }
            Self::InvalidRaw {
                expected,
                raw,
                source,
            } => write!(f, "身份 {expected} 的数字部分无法解析: {raw}: {source}"),
        }
    }
}

impl std::error::Error for IdParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidRaw { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdAllocatorError {
    MissingPrefix(&'static str),
    UnknownPrefix(String),
}

impl fmt::Display for IdAllocatorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingPrefix(prefix) => write!(f, "身份计数器缺少前缀 {prefix}"),
            Self::UnknownPrefix(prefix) => write!(f, "身份计数器包含未知前缀 {prefix}"),
        }
    }
}

impl std::error::Error for IdAllocatorError {}

fn parse_prefixed_id(input: &str, expected: &'static str) -> Result<u64, IdParseError> {
    let (prefix, raw) = input.rsplit_once('-').ok_or_else(|| IdParseError::MissingSeparator {
        expected,
        input: input.to_string(),
    })?;
    if prefix != expected {
        return Err(IdParseError::WrongPrefix {
            expected,
            found: prefix.to_string(),
        });
    }
    raw.parse::<u64>()
        .map_err(|source| IdParseError::InvalidRaw {
            expected,
            raw: raw.to_string(),
            source,
        })
}

pub trait StableId:
    Copy
    + Clone
    + Eq
    + Ord
    + std::hash::Hash
    + fmt::Debug
    + fmt::Display
    + FromStr<Err = IdParseError>
{
    const PREFIX: &'static str;

    fn from_raw(raw: u64) -> Self;

    fn raw(self) -> u64;
}

pub trait NextId: StableId + Sized {
    fn next(allocator: &mut IdAllocator) -> Self {
        allocator.next_id::<Self>()
    }
}

impl<T: StableId> NextId for T {}

macro_rules! define_stable_id {
    ($name:ident, $prefix:literal) => {
        #[derive(
            Debug,
            Clone,
            Copy,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
            Serialize,
            Deserialize,
        )]
        #[repr(transparent)]
        #[serde(transparent)]
        pub struct $name(u64);

        impl $name {
            pub const fn from_raw(raw: u64) -> Self {
                Self(raw)
            }

            pub const fn raw(self) -> u64 {
                self.0
            }
        }

        impl StableId for $name {
            const PREFIX: &'static str = $prefix;

            fn from_raw(raw: u64) -> Self {
                Self(raw)
            }

            fn raw(self) -> u64 {
                self.0
            }
        }

        impl FromStr for $name {
            type Err = IdParseError;

            fn from_str(input: &str) -> Result<Self, Self::Err> {
                parse_prefixed_id(input, <$name as StableId>::PREFIX).map(Self)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}-{:06}", <$name as StableId>::PREFIX, self.0)
            }
        }
    };
}

define_stable_id!(ActionId, "action");
define_stable_id!(ShotId, "shot");
define_stable_id!(PassId, "pass");
define_stable_id!(ContactId, "contact");
define_stable_id!(ProgramId, "program");
define_stable_id!(PenaltyId, "penalty");
define_stable_id!(EventId, "event");
define_stable_id!(ControlSequenceId, "control_sequence");
define_stable_id!(PossessionId, "possession");
define_stable_id!(PendingAdjudicationId, "pending_adjudication");
define_stable_id!(InteractionId, "interaction");
define_stable_id!(ReviewId, "review");
define_stable_id!(CheckpointId, "checkpoint");

pub const ID_PREFIXES: [&'static str; 13] = [
    ActionId::PREFIX,
    ShotId::PREFIX,
    PassId::PREFIX,
    ContactId::PREFIX,
    ProgramId::PREFIX,
    PenaltyId::PREFIX,
    EventId::PREFIX,
    ControlSequenceId::PREFIX,
    PossessionId::PREFIX,
    PendingAdjudicationId::PREFIX,
    InteractionId::PREFIX,
    ReviewId::PREFIX,
    CheckpointId::PREFIX,
];

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IdAllocator {
    counters: BTreeMap<String, u64>,
}

impl IdAllocator {
    pub fn new() -> Self {
        Self {
            counters: ID_PREFIXES
                .iter()
                .map(|prefix| ((*prefix).to_string(), 0))
                .collect(),
        }
    }

    pub fn from_counters(counters: BTreeMap<String, u64>) -> Result<Self, IdAllocatorError> {
        for prefix in ID_PREFIXES {
            if !counters.contains_key(prefix) {
                return Err(IdAllocatorError::MissingPrefix(prefix));
            }
        }
        for prefix in counters.keys() {
            if !ID_PREFIXES.contains(&prefix.as_str()) {
                return Err(IdAllocatorError::UnknownPrefix(prefix.clone()));
            }
        }
        Ok(Self { counters })
    }

    pub fn validate(&self) -> Result<(), IdAllocatorError> {
        Self::from_counters(self.counters.clone()).map(|_| ())
    }

    pub fn counters(&self) -> &BTreeMap<String, u64> {
        &self.counters
    }

    pub fn next(&mut self, prefix: &'static str) -> u64 {
        let counter = self
            .counters
            .get_mut(prefix)
            .expect("identity counter must exist");
        let next = counter
            .checked_add(1)
            .expect("identity counter overflowed");
        *counter = next;
        next
    }

    pub fn next_id<T: StableId>(&mut self) -> T {
        T::from_raw(self.next(T::PREFIX))
    }

    pub fn last<T: StableId>(&self) -> u64 {
        *self
            .counters
            .get(T::PREFIX)
            .expect("identity counter must exist")
    }

    pub fn observe<T: StableId>(&mut self, id: T) {
        let counter = self
            .counters
            .get_mut(T::PREFIX)
            .expect("identity counter must exist");
        if id.raw() > *counter {
            *counter = id.raw();
        }
    }

    pub fn reset<T: StableId>(&mut self) {
        let counter = self
            .counters
            .get_mut(T::PREFIX)
            .expect("identity counter must exist");
        *counter = 0;
    }
}

impl Default for IdAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl<'de> Deserialize<'de> for IdAllocator {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Counters {
            counters: BTreeMap<String, u64>,
        }

        let counters = Counters::deserialize(deserializer)?;
        IdAllocator::from_counters(counters.counters).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocator_issues_distinct_monotonic_ids() {
        let mut allocator = IdAllocator::new();
        let action = ActionId::next(&mut allocator);
        let shot = ShotId::next(&mut allocator);
        let next_action = ActionId::next(&mut allocator);
        assert_eq!(action.raw(), 1);
        assert_eq!(shot.raw(), 1);
        assert_eq!(next_action.raw(), 2);
        assert_eq!(allocator.last::<ActionId>(), 2);
        assert_eq!(allocator.last::<ShotId>(), 1);
    }

    #[test]
    fn display_and_from_str_round_trip() {
        let review = ReviewId::from_raw(42);
        assert_eq!(review.to_string(), "review-000042");
        assert_eq!(review.to_string().parse::<ReviewId>().unwrap(), review);
        assert!(matches!(
            "action-000042".parse::<ReviewId>(),
            Err(IdParseError::WrongPrefix { .. })
        ));
    }

    #[test]
    fn allocator_observes_and_resets_existing_identities() {
        let mut allocator = IdAllocator::new();
        allocator.observe(PassId::from_raw(99));
        assert_eq!(PassId::next(&mut allocator).raw(), 100);
        allocator.reset::<EventId>();
        assert_eq!(EventId::next(&mut allocator).raw(), 1);
        assert_eq!(allocator.last::<PassId>(), 100);
    }

    #[test]
    fn allocator_rejects_incomplete_counters() {
        let error = IdAllocator::from_counters(BTreeMap::new()).unwrap_err();
        assert!(matches!(error, IdAllocatorError::MissingPrefix(_)));
        let mut counters: BTreeMap<String, u64> = ID_PREFIXES
            .iter()
            .map(|prefix| ((*prefix).to_string(), 0))
            .collect();
        counters.insert("unknown".to_string(), 1);
        let error = IdAllocator::from_counters(counters).unwrap_err();
        assert!(matches!(error, IdAllocatorError::UnknownPrefix(_)));
    }

    #[test]
    fn serialized_allocator_validates_all_prefixes() {
        let allocator = IdAllocator::new();
        let value = serde_json::to_string(&allocator).unwrap();
        let restored: IdAllocator = serde_json::from_str(&value).unwrap();
        assert!(restored.validate().is_ok());
        let incomplete = r#"{"counters":{"action":1}}"#;
        let error = serde_json::from_str::<IdAllocator>(incomplete).unwrap_err();
        assert!(error.to_string().contains("缺少前缀"));
    }
}
