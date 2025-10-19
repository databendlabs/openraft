#[cfg(feature = "serde")]
mod serde_enabled {
    use crate::AppData;
    use crate::AppDataResponse;
    use crate::OptionalSerde;

    #[derive(Clone, Debug)]
    #[derive(serde::Serialize, serde::Deserialize)]
    #[derive(derive_more::Display)]
    struct SerdeEnabled {
        i: u32,
    }

    #[test]
    fn test_optional_serde_enabled() {
        /// A value that implements OptionalSerde implements serde::Serialize
        fn serde_it(v: impl OptionalSerde) {
            let s = serde_json::to_string(&v).unwrap();
            assert_eq!(r#"{"i":3}"#, s);
        }

        serde_it(SerdeEnabled { i: 3 })
    }

    #[test]
    fn test_app_data_serde_enabled() {
        /// A value that implements OptionalSerde implements serde::Serialize
        fn serde_it(v: impl AppData) {
            let s = serde_json::to_string(&v).unwrap();
            assert_eq!(r#"{"i":3}"#, s);
        }

        serde_it(SerdeEnabled { i: 3 })
    }

    #[test]
    fn test_app_data_response_serde_enabled() {
        /// A value that implements OptionalSerde implements serde::Serialize
        fn serde_it(v: impl AppDataResponse) {
            let s = serde_json::to_string(&v).unwrap();
            assert_eq!(r#"{"i":3}"#, s);
        }

        serde_it(SerdeEnabled { i: 3 })
    }
}

#[cfg(not(feature = "serde"))]
mod serde_disabled {

    use crate::AppData;
    use crate::AppDataResponse;
    use crate::OptionalSerde;

    #[derive(Clone, Debug)]
    #[derive(derive_more::Display)]
    struct SerdeDisabled {
        #[allow(dead_code)]
        i: u32,
    }

    #[test]
    fn test_optional_serde_disabled() {
        /// Any value implements
        fn accept_any_value(v: impl OptionalSerde) {
            let _ = v;
        }

        accept_any_value(SerdeDisabled { i: 3 });
        accept_any_value(1u32);
    }

    #[test]
    fn test_app_data_serde_disabled() {
        fn accept_any_value(v: impl AppData) {
            let _ = v;
        }

        accept_any_value(SerdeDisabled { i: 3 });
        accept_any_value(1u32);
    }

    #[test]
    fn test_app_data_response_serde_disabled() {
        fn accept_any_value(v: impl AppDataResponse) {
            let _ = v;
        }

        accept_any_value(SerdeDisabled { i: 3 });
        accept_any_value(1u32);
    }
}
