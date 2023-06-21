use core::fmt;
use std::ops::Deref;
use std::ops::DerefMut;

use crate::Instant;

/// Record the last update time for an object
#[derive(Debug)]
pub(crate) struct UTime<T, I: Instant> {
    data: T,
    utime: Option<I>,
}

impl<T: fmt::Display, I: Instant> fmt::Display for UTime<T, I> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.utime {
            Some(utime) => write!(f, "{}@{:?}", self.data, utime),
            None => write!(f, "{}", self.data),
        }
    }
}

impl<T: Clone, I: Instant> Clone for UTime<T, I> {
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
            utime: self.utime,
        }
    }
}

impl<T: Default, I: Instant> Default for UTime<T, I> {
    fn default() -> Self {
        Self {
            data: T::default(),
            utime: None,
        }
    }
}

impl<T: PartialEq, I: Instant> PartialEq for UTime<T, I> {
    fn eq(&self, other: &Self) -> bool {
        self.data == other.data && self.utime == other.utime
    }
}

impl<T: PartialEq + Eq, I: Instant> Eq for UTime<T, I> {}

impl<T, I: Instant> Deref for UTime<T, I> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl<T, I: Instant> DerefMut for UTime<T, I> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.data
    }
}

impl<T, I: Instant> UTime<T, I> {
    /// Creates a new object that keeps track of the time when it was last updated.
    pub(crate) fn new(now: I, data: T) -> Self {
        Self { data, utime: Some(now) }
    }

    /// Creates a new object that has no last-updated time.
    pub(crate) fn without_utime(data: T) -> Self {
        Self { data, utime: None }
    }

    /// Return the last updated time of this object.
    pub(crate) fn utime(&self) -> Option<I> {
        self.utime
    }

    /// Consumes this object and returns the inner data.
    pub(crate) fn into_inner(self) -> T {
        self.data
    }

    /// Update the content of the object and the last updated time.
    pub(crate) fn update(&mut self, now: I, data: T) {
        self.data = data;
        self.utime = Some(now);
    }

    /// Update the last updated time.
    pub(crate) fn touch(&mut self, now: I) {
        debug_assert!(
            Some(now) >= self.utime,
            "expect now: {:?}, must >= self.utime: {:?}, {:?}",
            now,
            self.utime,
            self.utime.unwrap() - now,
        );
        self.utime = Some(now);
    }
}
