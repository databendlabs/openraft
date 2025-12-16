use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

#[derive(Debug, Clone)]
pub(crate) struct SharedIdGenerator {
    next_id: Arc<AtomicU64>,
}

impl Default for SharedIdGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl PartialEq for SharedIdGenerator {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.next_id, &other.next_id)
    }
}

impl Eq for SharedIdGenerator {}

impl SharedIdGenerator {
    pub(crate) fn new() -> Self {
        Self {
            next_id: Arc::new(AtomicU64::new(1)),
        }
    }
    pub(crate) fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_next_id_sequential() {
        let id_gen = SharedIdGenerator::new();

        assert_eq!(1, id_gen.next_id());
        assert_eq!(2, id_gen.next_id());
        assert_eq!(3, id_gen.next_id());
    }

    #[test]
    fn test_clones_share_counter() {
        let id_gen1 = SharedIdGenerator::new();
        let id_gen2 = id_gen1.clone();

        assert_eq!(1, id_gen1.next_id());
        assert_eq!(2, id_gen2.next_id());
        assert_eq!(3, id_gen1.next_id());
    }

    #[test]
    fn test_partial_eq() {
        let id_gen1 = SharedIdGenerator::new();
        let id_gen2 = id_gen1.clone();
        let id_gen3 = SharedIdGenerator::new();

        assert_eq!(id_gen1, id_gen2);
        assert_ne!(id_gen1, id_gen3);
    }

    #[test]
    fn test_concurrent_access() {
        use std::thread;

        let id_gen = SharedIdGenerator::new();
        let mut handles = vec![];

        for _ in 0..10 {
            let g = id_gen.clone();
            handles.push(thread::spawn(move || (0..100).map(|_| g.next_id()).collect::<Vec<_>>()));
        }

        let mut all_ids: Vec<u64> = handles.into_iter().flat_map(|h| h.join().unwrap()).collect();

        all_ids.sort();

        // All IDs should be unique and sequential from 1 to 1000
        let expected: Vec<u64> = (1..=1000).collect();
        assert_eq!(expected, all_ids);
    }
}
