//! Widget and Window IDs.

use std::borrow;
use std::hash::Hash;
use std::sync::atomic::{self, AtomicU64};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum A11yId {
    Window(u64),
    Widget(Id),
}

// impl A11yId {
//     pub fn new_widget() -> Self {
//         Self::Widget(Id::unique())
//     }

//     pub fn new_window() -> Self {
//         Self::Window(window_node_id())
//     }
// }

impl From<u64> for A11yId {
    fn from(id: u64) -> Self {
        Self::Window(id)
    }
}

impl From<Id> for A11yId {
    fn from(id: Id) -> Self {
        assert!(!matches!(id.0, Internal::Set(_)));
        Self::Widget(id)
    }
}

impl IdEq for A11yId {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (A11yId::Widget(self_), A11yId::Widget(other)) => {
                IdEq::eq(self_, other)
            }
            _ => self == other,
        }
    }
}

impl From<accesskit::NodeId> for A11yId {
    fn from(value: accesskit::NodeId) -> Self {
        let val = u64::from(value.0);
        if val > u32::MAX as u64 {
            Self::Window(value.0)
        } else {
            Self::Widget(Id::from(val as u64))
        }
    }
}

impl From<A11yId> for accesskit::NodeId {
    fn from(value: A11yId) -> Self {
        let node_id = match value {
            A11yId::Window(id) => id,
            A11yId::Widget(id) => id.into(),
        };
        accesskit::NodeId(node_id)
    }
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_WINDOW_ID: AtomicU64 = AtomicU64::new(1);

/// The identifier of a generic widget.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Id(pub Internal);

impl Id {
    /// Creates a custom [`Id`].
    pub fn new(id: impl Into<borrow::Cow<'static, str>>) -> Self {
        Self(Internal::Custom(Self::next(), id.into()))
    }

    /// resets the id counter
    pub fn reset() {
        NEXT_ID.store(1, atomic::Ordering::Relaxed);
    }

    fn next() -> u64 {
        NEXT_ID.fetch_add(1, atomic::Ordering::Relaxed)
    }

    /// Creates a unique [`Id`].
    ///
    /// This function produces a different [`Id`] every time it is called.
    pub fn unique() -> Self {
        let id = Self::next();

        Self(Internal::Unique(id))
    }
}

impl IdEq for Id {
    fn eq(&self, other: &Self) -> bool {
        IdEq::eq(&self.0, &other.0)
    }
}
// Not meant to be used directly
impl From<u64> for Id {
    fn from(value: u64) -> Self {
        Self(Internal::Unique(value))
    }
}

// Not meant to be used directly
impl From<Id> for u64 {
    fn from(val: Id) -> u64 {
        match &val.0 {
            Internal::Unique(id) => *id,
            Internal::Custom(id, _) => *id,
            // this is a set id, which is not a valid id and will not ever be converted to a NonZeroU128
            // so we panic
            Internal::Set(_) => {
                panic!("Cannot convert a set id to a NonZeroU128")
            }
        }
    }
}

impl ToString for Id {
    fn to_string(&self) -> String {
        match &self.0 {
            Internal::Unique(_) => "Undefined".to_string(),
            Internal::Custom(_, id) => id.to_string(),
            Internal::Set(_) => "Set".to_string(),
        }
    }
}

// XXX WIndow IDs are made unique by adding u32::MAX to them
/// get window node id that won't conflict with other node ids for the duration of the program
pub fn window_node_id() -> u64 {
    u32::MAX as u64
        + NEXT_WINDOW_ID.fetch_add(1, atomic::Ordering::Relaxed) as u64
}

// TODO refactor to make panic impossible?
#[derive(Debug, Clone, Eq)]
/// Internal representation of an [`Id`].
pub enum Internal {
    /// a unique id
    Unique(u64),
    /// a custom id, which is equal to any [`Id`] with a matching number or string
    Custom(u64, borrow::Cow<'static, str>),
    /// XXX Do not use this as an id for an accessibility node, it will panic!
    /// XXX Only meant to be used for widgets that have multiple accessibility nodes, each with a
    /// unique or custom id
    /// an Id Set, which is equal to any [`Id`] with a matching number or string
    Set(Vec<Self>),
}

impl PartialEq for Internal {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Unique(l0), Self::Unique(r0)) => l0 == r0,
            (Self::Custom(_, l1), Self::Custom(_, r1)) => l1 == r1,
            (Self::Set(l0), Self::Set(r0)) => l0 == r0,
            _ => false,
        }
    }
}

/// Similar to PartialEq, but only intended for use when comparing Ids
pub trait IdEq {
    fn eq(&self, other: &Self) -> bool;
}

impl IdEq for Internal {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Unique(l0), Self::Unique(r0)) => l0 == r0,
            (Self::Custom(l0, l1), Self::Custom(r0, r1)) => {
                l0 == r0 || l1 == r1
            }
            // allow custom ids to be equal to unique ids
            (Self::Unique(l0), Self::Custom(r0, _))
            | (Self::Custom(l0, _), Self::Unique(r0)) => l0 == r0,
            (Self::Set(l0), Self::Set(r0)) => l0 == r0,
            // allow set ids to just be equal to any of their members
            (Self::Set(l0), r) | (r, Self::Set(l0)) => {
                l0.iter().any(|l| l == r)
            }
        }
    }
}

impl Hash for Internal {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            Self::Unique(id) => id.hash(state),
            Self::Custom(name, _) => name.hash(state),
            Self::Set(ids) => ids.hash(state),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Id;

    #[test]
    fn unique_generates_different_ids() {
        let a = Id::unique();
        let b = Id::unique();

        assert_ne!(a, b);
    }
}
