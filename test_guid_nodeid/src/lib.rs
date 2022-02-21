#[cfg(test)]
mod tests {
    #[test]
    fn ensure_guid_nodeid_works() {
        let _guid_nodeid = openraft::NodeId::new_from_parts(1, 2);
    }
}
