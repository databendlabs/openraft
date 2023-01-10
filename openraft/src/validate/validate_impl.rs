use std::error::Error;

use crate::validate::Validate;

/// Dummy impl Validate for primitive types
macro_rules! impl_validate {
    ($typ: ty) => {
        impl Validate for $typ {
            fn validate(&self) -> Result<(), Box<dyn Error>> {
                Ok(())
            }
        }
    };
}

impl_validate!(bool);
impl_validate!(usize);
impl_validate!(isize);
impl_validate!(u8);
impl_validate!(u16);
impl_validate!(u32);
impl_validate!(u64);
impl_validate!(i8);
impl_validate!(i16);
impl_validate!(i32);
impl_validate!(i64);
impl_validate!(&str);
impl_validate!(String);
