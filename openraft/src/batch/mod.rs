//! A container that stores elements efficiently by avoiding heap allocation for single elements.

mod batch_trait;
pub mod inline_batch;

#[allow(unused)]
pub use batch_trait::Batch;
pub use inline_batch::InlineBatch;
#[cfg(test)]
mod tests {
    use crate::batch::InlineBatch;
    fn batch<T>(items: impl IntoIterator<Item = T>) -> InlineBatch<T> {
        items.into_iter().collect()
    }

    #[test]
    fn test_collect() {
        assert_eq!(batch([42]).as_ref(), &[42]);
        assert_eq!(batch([1, 2, 3]).as_ref(), &[1, 2, 3]);
        assert_eq!(batch::<i32>([]).as_ref(), &[] as &[i32]);
    }

    #[test]
    fn test_extend() {
        let mut v = batch([1]);
        v.extend([2]);
        assert_eq!(v.as_ref(), &[1, 2]);

        let mut v = batch([1]);
        v.extend([2, 3]);
        assert_eq!(v.as_ref(), &[1, 2, 3]);

        let mut v = batch([1, 2]);
        v.extend([3]);
        assert_eq!(v.as_ref(), &[1, 2, 3]);

        let mut v = batch([1, 2]);
        v.extend([3, 4]);
        assert_eq!(v.as_ref(), &[1, 2, 3, 4]);
    }

    #[test]
    fn test_clone() {
        assert_eq!(batch([42]).clone(), batch([42]));
        assert_eq!(batch([1, 2]).clone(), batch([1, 2]));
    }

    #[test]
    fn test_empty() {
        assert_eq!(batch::<i32>([]).as_ref(), &[] as &[i32]);
    }
}
