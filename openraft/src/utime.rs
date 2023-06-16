use core::fmt;
use std::ops::Deref;
use std::ops::DerefMut;
use std::time::Instant;

/// Record the last update time for an object
#[derive(Debug, Default)]
pub(crate) struct UTime<T> {
    data: T,
    utime: Option<Instant>,
}

impl<T: fmt::Display> fmt::Display for UTime<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.utime {
            Some(utime) => write!(f, "{}@{:?}", self.data, utime),
            None => write!(f, "{}", self.data),
        }
    }
}

impl<T: Clone> Clone for UTime<T> {
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
            utime: self.utime,
        }
    }
}

impl<T: PartialEq> PartialEq for UTime<T> {
    fn eq(&self, other: &Self) -> bool {
        self.data == other.data && self.utime == other.utime
    }
}

impl<T: PartialEq + Eq> Eq for UTime<T> {}

impl<T> Deref for UTime<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl<T> DerefMut for UTime<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.data
    }
}

impl<T> UTime<T> {
    /// Creates a new object that keeps track of the time when it was last updated.
    pub(crate) fn new(now: Instant, data: T) -> Self {
        Self { data, utime: Some(now) }
    }

    /// Creates a new object that has no last-updated time.
    pub(crate) fn without_utime(data: T) -> Self {
        Self { data, utime: None }
    }

    /// Return the last updated time of this object.
    pub(crate) fn utime(&self) -> Option<Instant> {
        self.utime
    }

    /// Consumes this object and returns the inner data.
    pub(crate) fn into_inner(self) -> T {
        self.data
    }

    /// Update the content of the object and the last updated time.
    pub(crate) fn update(&mut self, now: Instant, data: T) {
        self.data = data;
        self.utime = Some(now);
    }

    /// Update the last updated time.
    pub(crate) fn touch(&mut self, now: Instant) {
        debug_assert!(
            Some(now) >= self.utime,
            "expect now: {:?}, must >= self.utime: {:?}, {:?}",
            now,
            self.utime,
            self.utime.unwrap() - now
        );
        self.utime = Some(now);
    }
}
