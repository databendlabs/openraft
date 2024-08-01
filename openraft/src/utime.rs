use core::fmt;
use std::ops::Deref;
use std::ops::DerefMut;
use std::time::Duration;

use crate::display_ext::DisplayInstantExt;
use crate::Instant;

/// Stores an object along with its last update time and lease duration.
/// The lease duration specifies how long the object remains valid.
#[derive(Debug, Clone)]
#[derive(PartialEq, Eq)]
pub(crate) struct Leased<T, I: Instant> {
    data: T,
    last_update: Option<I>,
    lease: Duration,
}

impl<T: fmt::Display, I: Instant> fmt::Display for Leased<T, I> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.last_update {
            Some(utime) => write!(f, "{}@{}+{:?}", self.data, utime.display(), self.lease),
            None => write!(f, "{}", self.data),
        }
    }
}

impl<T: Default, I: Instant> Default for Leased<T, I> {
    fn default() -> Self {
        Self {
            data: T::default(),
            last_update: None,
            lease: Default::default(),
        }
    }
}

impl<T, I: Instant> Deref for Leased<T, I> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl<T, I: Instant> DerefMut for Leased<T, I> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.data
    }
}

impl<T, I: Instant> Leased<T, I> {
    /// Creates a new object that keeps track of the time when it was last updated.
    pub(crate) fn new(now: I, lease: Duration, data: T) -> Self {
        Self {
            data,
            last_update: Some(now),
            lease,
        }
    }

    /// Creates a new object that has no last-updated time.
    #[allow(dead_code)]
    pub(crate) fn without_last_update(data: T) -> Self {
        Self {
            data,
            last_update: None,
            lease: Duration::default(),
        }
    }

    /// Return the last updated time of this object.
    pub(crate) fn last_update(&self) -> Option<I> {
        self.last_update
    }

    /// Return a Display instance that shows the last updated time and lease duration relative to
    /// `now`.
    pub(crate) fn time_info(&self, now: I) -> impl fmt::Display + '_ {
        struct DisplayTimeInfo<'a, T, I: Instant> {
            now: I,
            leased: &'a Leased<T, I>,
        }

        impl<'a, T, I> fmt::Display for DisplayTimeInfo<'a, T, I>
        where I: Instant
        {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                match &self.leased.last_update {
                    Some(utime) => {
                        let expire_at = *utime + self.leased.lease;
                        write!(
                            f,
                            "now: {}, last_update: {}({:?} ago), lease: {:?}, expire at: {}({:?} since now)",
                            self.now.display(),
                            utime.display(),
                            self.now.saturating_duration_since(*utime),
                            self.leased.lease,
                            expire_at.display(),
                            expire_at.saturating_duration_since(self.now)
                        )
                    }
                    None => {
                        write!(f, "last_update: None")
                    }
                }
            }
        }

        DisplayTimeInfo { now, leased: self }
    }

    /// Consumes this object and returns the inner data.
    #[allow(dead_code)]
    pub(crate) fn into_inner(self) -> T {
        self.data
    }

    /// Update the content of the object and the last updated time.
    pub(crate) fn update(&mut self, now: I, lease: Duration, data: T) {
        self.data = data;
        self.last_update = Some(now);
        self.lease = lease;
    }

    /// Reset the lease duration, so that the object expire at once.
    #[allow(dead_code)]
    pub(crate) fn reset_lease(&mut self) {
        self.lease = Duration::default();
    }

    /// Checks if the value is expired based on the provided `now` timestamp.
    /// An additional `timeout` parameter can be used to extend the lease under various situations.
    pub(crate) fn is_expired(&self, now: I, timeout: Duration) -> bool {
        match self.last_update {
            Some(utime) => now > utime + self.lease + timeout,
            None => true,
        }
    }

    /// Update the last updated time.
    pub(crate) fn touch(&mut self, now: I, lease: Duration) {
        debug_assert!(
            Some(now) >= self.last_update,
            "expect now: {}, must >= self.utime: {}, {:?}",
            now.display(),
            self.last_update.unwrap().display(),
            self.last_update.unwrap() - now,
        );
        self.last_update = Some(now);
        self.lease = lease;
    }
}
