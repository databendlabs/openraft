use std::error::Error;
use std::fmt::Debug;
use std::fmt::Display;
use std::fmt::Formatter;
use std::ops::Deref;
use std::ops::DerefMut;

// TODO: add doc when these validation APIs are stable.
#[doc(hidden)]
#[macro_export]
macro_rules! less {
    ($a: expr, $b: expr) => {{
        let a = $a;
        let b = $b;
        if (a < b) {
            // Ok
        } else {
            Err(::anyerror::AnyError::error(format!(
                "expect: {}({:?}) {} {}({:?})",
                stringify!($a),
                a,
                "<",
                stringify!($b),
                b,
            )))?;
        }
    }};
}

#[doc(hidden)]
#[macro_export]
macro_rules! less_equal {
    ($a: expr, $b: expr) => {{
        let a = $a;
        let b = $b;
        if (a <= b) {
            // Ok
        } else {
            Err(::anyerror::AnyError::error(format!(
                "expect: {}({:?}) {} {}({:?})",
                stringify!($a),
                a,
                "<=",
                stringify!($b),
                b,
            )))?;
        }
    }};
}

#[doc(hidden)]
#[macro_export]
macro_rules! equal {
    ($a: expr, $b: expr) => {{
        let a = $a;
        let b = $b;
        if (a == b) {
            // Ok
        } else {
            Err(::anyerror::AnyError::error(format!(
                "expect: {}({:?}) {} {}({:?})",
                stringify!($a),
                a,
                "==",
                stringify!($b),
                b,
            )))?;
        }
    }};
}

/// A type that validates its internal state.
///
/// An example of defining field `a` whose value must not exceed `10`.
/// ```ignore
/// # use std::error::Error;
/// # use openraft::less_equal;
/// struct Foo { a: u64 }
/// impl Validate for Foo {
///     fn validate(&self) -> Result<(), Box<dyn Error>> {
///         less_equal!(self.a, 10);
///         Ok(())
///     }
/// }
/// ```
pub(crate) trait Validate {
    fn validate(&self) -> Result<(), Box<dyn Error>>;
}

impl<T: Validate> Validate for &T {
    fn validate(&self) -> Result<(), Box<dyn Error>> {
        (*self).validate()
    }
}

/// A wrapper of T that validate the state of T every time accessing it.
///
/// - It validates the state before accessing it, i.e., if when a invalid state is written to it, it
///   won't panic until next time accessing it.
/// - The validation is turned on only when `debug_assertions` is enabled.
///
/// An example of defining field `a` whose value must not exceed `10`.
/// ```ignore
/// # use std::error::Error;
/// # use openraft::less_equal;
/// struct Foo { a: u64 }
/// impl Validate for Foo {
///     fn validate(&self) -> Result<(), Box<dyn Error>> {
///         less_equal!(self.a, 10);
///         Ok(())
///     }
/// }
///
/// let f = Valid::new(Foo { a: 20 });
/// let _x = f.a; // panic: panicked at 'invalid state: expect: self.a(20) <= 10(10) ...
/// ```
pub(crate) struct Valid<T>
where T: Validate
{
    pub(crate) enable_validate: bool,
    inner: T,
}

impl<T: Validate> Valid<T> {
    pub(crate) fn new(inner: T) -> Self {
        Self {
            enable_validate: true,
            inner,
        }
    }
}

impl<T> Deref for Valid<T>
where T: Validate
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        #[cfg(debug_assertions)]
        if self.enable_validate {
            if let Err(e) = self.inner.validate() {
                panic!("invalid state: {}", e);
            }
        }

        &self.inner
    }
}

impl<T> DerefMut for Valid<T>
where T: Validate
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        #[cfg(debug_assertions)]
        if self.enable_validate {
            if let Err(e) = self.inner.validate() {
                panic!("invalid state: {}", e);
            }
        }

        &mut self.inner
    }
}

impl<T: PartialEq> PartialEq for Valid<T>
where T: Validate
{
    fn eq(&self, other: &Self) -> bool {
        self.inner.eq(&other.inner)
    }
}

impl<T: Eq> Eq for Valid<T> where T: Validate {}

impl<T: Debug> Debug for Valid<T>
where T: Validate
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.inner.fmt(f)
    }
}

impl<T: Clone> Clone for Valid<T>
where T: Validate
{
    fn clone(&self) -> Self {
        Self {
            enable_validate: self.enable_validate,
            inner: self.inner.clone(),
        }
    }
}

impl<T: Default> Default for Valid<T>
where T: Validate
{
    fn default() -> Self {
        Self {
            enable_validate: true,
            inner: T::default(),
        }
    }
}

impl<T: Display> Display for Valid<T>
where T: Validate
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.inner.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fmt::Display;
    use std::fmt::Formatter;

    use crate::validate::Valid;
    use crate::validate::Validate;

    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    struct Foo {
        a: u64,
    }

    impl Validate for Foo {
        fn validate(&self) -> Result<(), Box<dyn Error>> {
            less_equal!(self.a, 10);
            Ok(())
        }
    }

    impl Display for Foo {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            write!(f, "Foo:{}", self.a)
        }
    }

    #[test]
    fn test_trait() {
        // Display
        println!("Display: {}", Valid::new(Foo { a: 3 }));

        // Default
        assert_eq!(Valid::new(Foo { a: 0 }), Valid::<Foo>::default(), "impl Default");

        // Clone
        #[allow(clippy::redundant_clone)]
        let _clone = Valid::new(Foo { a: 3 }).clone();
    }

    #[test]
    fn test_validate() {
        // panic when reading an invalid state
        let res = std::panic::catch_unwind(|| {
            let f = Valid::new(Foo { a: 20 });
            let _x = f.a;
        });
        tracing::info!("res: {:?}", res);
        assert!(res.is_err());

        // Disable validation
        let res = std::panic::catch_unwind(|| {
            let mut f = Valid::new(Foo { a: 20 });
            f.enable_validate = false;
            let _x = f.a;
        });
        tracing::info!("res: {:?}", res);
        assert!(res.is_ok());

        // valid state
        let res = std::panic::catch_unwind(|| {
            let f = Valid::new(Foo { a: 10 });
            let _x = f.a;
        });
        assert!(res.is_ok());

        // no panic when just becoming invalid
        let res = std::panic::catch_unwind(|| {
            let mut f = Valid::new(Foo { a: 10 });
            f.a += 3;
        });
        assert!(res.is_ok());

        // panic on next write access
        let res = std::panic::catch_unwind(|| {
            let mut f = Valid::new(Foo { a: 10 });
            f.a += 3;
            f.a += 1;
        });
        tracing::info!("res: {:?}", res);
        assert!(res.is_err());
    }

    #[test]
    fn test_validate_impl_for_ref() {
        // Valid
        let res = std::panic::catch_unwind(|| {
            let f = Foo { a: 5 };
            let f = Valid::new(&f);
            let _x = f.a;
        });
        tracing::info!("res: {:?}", res);
        assert!(res.is_ok());

        // Invalid
        let res = std::panic::catch_unwind(|| {
            let f = Foo { a: 20 };
            let f = Valid::new(&f);
            let _x = f.a;
        });
        tracing::info!("res: {:?}", res);
        assert!(res.is_err());
    }
}
