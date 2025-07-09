use serde::{Deserialize, Serialize};

/// bitflag which basically switches on visibility:<something> tags
/// it's an AND operation for every bit set
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibilityFilter(pub u32);

impl VisibilityFilter {
    // bit positions
    pub const PUBLIC: u32 = 1 << 0;
    pub const UNINDEXED: u32 = 1 << 1;
    pub const PRIVATE: u32 = 1 << 2;

    pub fn new(bits: u32) -> Self {
        Self(bits)
    }

    pub fn includes_public(&self) -> bool {
        self.0 & Self::PUBLIC != 0
    }

    pub fn includes_unindexed(&self) -> bool {
        self.0 & Self::UNINDEXED != 0
    }

    pub fn includes_private(&self) -> bool {
        self.0 & Self::PRIVATE != 0
    }

    pub fn for_user(authenticated: bool, is_search: bool) -> Self {
        match (authenticated, is_search) {
            (true, true) => Self::new(Self::PUBLIC | Self::PRIVATE),
            (true, false) => {
                Self::new(Self::PUBLIC | Self::UNINDEXED | Self::PRIVATE)
            }
            (false, true) => Self::new(Self::PUBLIC),
            (false, false) => Self::new(Self::PUBLIC),
        }
    }
}

impl Default for VisibilityFilter {
    fn default() -> Self {
        // maybe this should rely on the toml
        Self::new(Self::PUBLIC)
    }
}
