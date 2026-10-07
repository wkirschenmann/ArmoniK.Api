//! Point 2: a matcher and a cell cheap enough to ask on every `sometimes` event.
//!
//! `Targets::would_enable` and `ArcSwap::load` each cost tens of nanoseconds here (see
//! examples/parts.rs). The matcher below is the same rule - the longest target prefix decides, the
//! default level for the rest - over a short vector; the cell hands out a plain reference and keeps
//! every filter it ever held until it is dropped, so that a reader needs no guard.

use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Mutex, PoisonError};

use tracing::Level;
use tracing_core::LevelFilter;
use tracing_subscriber::filter::Targets;

pub struct FastFilter {
    /// Longest target first.
    directives: Vec<(Box<str>, LevelFilter)>,
    default: LevelFilter,
    max: LevelFilter,
}

impl FastFilter {
    pub fn from_targets(targets: &Targets) -> Self {
        let mut directives: Vec<(Box<str>, LevelFilter)> = targets
            .iter()
            .map(|(target, level)| (target.into(), level))
            .collect();
        directives.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        let default = targets.default_level().unwrap_or(LevelFilter::OFF);
        let max = directives
            .iter()
            .map(|(_, level)| *level)
            .chain([default])
            .max()
            .unwrap_or(LevelFilter::OFF);
        FastFilter {
            directives,
            default,
            max,
        }
    }

    pub fn off() -> Self {
        FastFilter {
            directives: Vec::new(),
            default: LevelFilter::OFF,
            max: LevelFilter::OFF,
        }
    }

    #[inline]
    pub fn would_enable(&self, target: &str, level: &Level) -> bool {
        for (prefix, allowed) in &self.directives {
            if target.starts_with(&**prefix) {
                return level <= allowed;
            }
        }
        level <= &self.default
    }

    pub fn max_level(&self) -> LevelFilter {
        self.max
    }
}

pub struct FilterCell {
    current: AtomicPtr<FastFilter>,
    /// Every filter replaced: a reader may still hold one, and none is freed before the cell.
    retired: Mutex<Vec<Box<FastFilter>>>,
}

impl FilterCell {
    pub fn new(filter: FastFilter) -> Self {
        FilterCell {
            current: AtomicPtr::new(Box::into_raw(Box::new(filter))),
            retired: Mutex::new(Vec::new()),
        }
    }

    #[inline]
    pub fn get(&self) -> &FastFilter {
        // Never freed while `self` lives.
        unsafe { &*self.current.load(Ordering::Acquire) }
    }

    pub fn set(&self, filter: FastFilter) {
        let old = self
            .current
            .swap(Box::into_raw(Box::new(filter)), Ordering::AcqRel);
        self.retired
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(unsafe { Box::from_raw(old) });
    }

    pub fn retired_len(&self) -> usize {
        self.retired.lock().unwrap_or_else(PoisonError::into_inner).len()
    }
}

impl Drop for FilterCell {
    fn drop(&mut self) {
        drop(unsafe { Box::from_raw(*self.current.get_mut()) });
    }
}
